//! The pi engine sidecar: how the daemon spawns `pi --mode rpc` confined
//! (`pi-agent-adoption.md` §D).
//!
//! The supervisor ([`crate::supervisor`]) drives the engine through the
//! [`SpawnEngine`](crate::supervisor::SpawnEngine) trait; this module builds the
//! CONFINED SPAWN for the real pi process. It is deliberately split into a pure,
//! unit-tested confinement + argv builder here and the `run_once` spawn (the
//! actual process launch + stdio wiring + the session-token handoff, a later
//! slice), so the security-relevant sandbox SHAPE is testable without spawning.
//!
//! Confinement: read-only base (`/usr` + the node runtime + the pi install), no
//! network at all, a writable tmpfs `/tmp` + the pi state dir, and the daemon's
//! CONTRACT socket bound read-write at a fixed in-sandbox path the plugins
//! connect to. That contract socket is pi's only channel out: the plugins ask
//! this daemon (Authorize/Report/Execute) and the DAEMON makes the model call
//! through `ProxiedProvider`. The ai-proxy socket is bound only if it exists -
//! the proxy serves D-Bus and binds no socket today, and bwrap refuses to start
//! on a missing bind source, so an unconditional bind killed every spawn. The exact base bind set (node also needs its
//! dynamic loader) is confirmed and tightened by the `run_once` `#[ignore]d`
//! spawn test against a real node, the way arlen-run validates its own argv.

use crate::supervisor::{EngineExit, SpawnEngine};
use arlen_confiner::{app_runtime_profile, Bind, Confinement, ConfinerError, NetworkPolicy};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{error, warn};

/// The fixed in-sandbox path the daemon's contract socket is bound to. The host
/// path varies with `XDG_RUNTIME_DIR`; the sandbox sees a stable path, and the
/// plugins read it from `ARLEN_AI_ENGINE_SOCKET`.
pub const SANDBOX_CONTRACT_SOCKET: &str = "/run/arlen/ai-engine.sock";
/// The fixed in-sandbox path the ai-proxy socket is bound to (pi's only egress).
pub const SANDBOX_PROXY_SOCKET: &str = "/run/arlen/ai-proxy.sock";

/// The session-token file's name, in the pi state dir (bound read-write at its
/// host path, so the in-sandbox path equals the host path). The `run_once` spawn
/// writes the per-run token there `0600` and points `ARLEN_AI_ENGINE_TOKEN_FILE`
/// at it, so the secret reaches pi WITHOUT ever entering the bwrap argv (where a
/// same-uid `/proc/<pid>/cmdline` reader would see it).
pub const SESSION_TOKEN_FILENAME: &str = ".arlen-session-token";

/// The host paths the sidecar spawn needs, resolved by the caller from config/
/// env (fail-closed, no invented defaults). All must be absolute.
#[derive(Debug, Clone)]
pub struct SidecarPaths {
    /// The node (>=22) runtime directory, bound read-only (e.g.
    /// `~/.local/share/arlen-node22`).
    pub node_runtime: String,
    /// The node binary to exec (under `node_runtime`, e.g. `<node_runtime>/bin/node`).
    pub node_bin: String,
    /// The pi install directory, bound read-only (the repo or the packaged dir).
    pub pi_install: String,
    /// The pi CLI entry passed to node (e.g.
    /// `<pi_install>/packages/coding-agent/dist/cli.js`).
    pub pi_cli: String,
    /// The pi writable state dir, bound read-write (its cache/work + the
    /// in-sandbox `HOME`).
    pub pi_state: String,
    /// The host path of the daemon's contract socket, bound to
    /// [`SANDBOX_CONTRACT_SOCKET`].
    pub contract_socket: String,
    /// The host path of the ai-proxy socket, bound to [`SANDBOX_PROXY_SOCKET`].
    pub proxy_socket: String,
    /// The host path of the built Arlen pi extension entry (the gate + audit
    /// shims, e.g. `<install>/pi-plugins/dist/index.js`). Its directory is bound
    /// read-only and the path is passed to pi via `--extension`. The extension
    /// imports only Node built-ins, so no node_modules need be in the sandbox.
    pub arlen_extension: String,
}

/// The pi CLI entry under the install dir (verified: the `coding-agent` package
/// bin, which the `profile:rpc` script runs `--mode rpc`).
const PI_CLI_REL: &str = "packages/coding-agent/dist/cli.js";

impl SidecarPaths {
    /// Resolve the sidecar paths from the environment, fail-closed. `getenv`
    /// abstracts `std::env::var` so resolution is unit-tested without mutating
    /// the process environment (racy under parallel tests).
    ///
    /// `ARLEN_PI_NODE_RUNTIME` (the node >=22 runtime dir) and `ARLEN_PI_INSTALL`
    /// (the pi install dir) are REQUIRED with no invented default - a missing one
    /// is an error, never a guessed path (the forage `ForageBuildConfig`
    /// precedent: the daemon refuses to spawn pi rather than dial a wrong path).
    /// `node_bin`/`pi_cli` derive from those; `pi_state` is
    /// `$XDG_STATE_HOME|$HOME/.local/state` + `arlen/pi`; the sockets are the
    /// daemon's own contract socket and the ai-proxy socket under the runtime
    /// dir. Every resolved path must be absolute.
    pub fn resolve(
        getenv: impl Fn(&str) -> Option<String>,
        contract_socket: String,
    ) -> Result<Self, String> {
        let node_runtime = getenv("ARLEN_PI_NODE_RUNTIME")
            .filter(|s| !s.is_empty())
            .ok_or("ARLEN_PI_NODE_RUNTIME is not set (the node>=22 runtime dir for the pi sidecar)")?;
        let pi_install = getenv("ARLEN_PI_INSTALL")
            .filter(|s| !s.is_empty())
            .ok_or("ARLEN_PI_INSTALL is not set (the pi install dir for the sidecar)")?;
        let arlen_extension = getenv("ARLEN_PI_EXTENSION")
            .filter(|s| !s.is_empty())
            .ok_or("ARLEN_PI_EXTENSION is not set (the built Arlen pi extension entry)")?;

        let runtime_dir = getenv("XDG_RUNTIME_DIR")
            .filter(|s| !s.is_empty())
            .ok_or("XDG_RUNTIME_DIR is not set (needed for the ai-proxy socket path)")?;
        let state_home = getenv("XDG_STATE_HOME")
            .filter(|s| !s.is_empty())
            .or_else(|| getenv("HOME").filter(|s| !s.is_empty()).map(|h| format!("{h}/.local/state")))
            .ok_or("neither XDG_STATE_HOME nor HOME is set (needed for the pi state dir)")?;

        let paths = SidecarPaths {
            node_bin: format!("{}/bin/node", node_runtime.trim_end_matches('/')),
            pi_cli: format!("{}/{PI_CLI_REL}", pi_install.trim_end_matches('/')),
            pi_state: format!("{}/arlen/pi", state_home.trim_end_matches('/')),
            proxy_socket: format!("{}/arlen/ai-proxy.sock", runtime_dir.trim_end_matches('/')),
            node_runtime,
            pi_install,
            contract_socket,
            arlen_extension,
        };
        // Every path the confinement binds or execs must be absolute; surface a
        // bad one here rather than at spawn.
        for (label, p) in [
            ("node runtime", &paths.node_runtime),
            ("node bin", &paths.node_bin),
            ("pi install", &paths.pi_install),
            ("pi cli", &paths.pi_cli),
            ("pi state", &paths.pi_state),
            ("contract socket", &paths.contract_socket),
            ("ai-proxy socket", &paths.proxy_socket),
            ("arlen extension", &paths.arlen_extension),
        ] {
            if !Path::new(p).is_absolute() {
                return Err(format!("the resolved {label} path is not absolute: {p}"));
            }
        }
        Ok(paths)
    }
}

/// Reject a non-absolute path (bwrap requires absolute binds; a relative source
/// is ambiguous). UTF-8 is already guaranteed by the `String` fields.
fn require_abs(path: &str) -> Result<String, ConfinerError> {
    if Path::new(path).is_absolute() {
        Ok(path.to_string())
    } else {
        Err(ConfinerError::RelativePath(path.to_string()))
    }
}

/// Build the pi sidecar's confinement: read-only `/usr` + node + pi, the pi
/// state dir writable, the contract + proxy sockets bound read-write at their
/// fixed sandbox paths, no network. The non-secret environment points the
/// plugins at the in-sandbox socket paths and pins `HOME` at the state dir so
/// node/pi write their caches there, never the host home.
///
/// `token_file` is the in-sandbox PATH of the 0600 session-token file (not the
/// token itself): when `Some`, `ARLEN_AI_ENGINE_TOKEN_FILE` points the plugins
/// at it. The secret token stays out of this (loggable) builder AND out of the
/// bwrap argv; only its file path - not secret - is set. `None` builds the
/// token-free shape the unit tests assert on.
pub fn pi_sidecar_confinement(
    paths: &SidecarPaths,
    token_file: Option<&str>,
) -> Result<Confinement, ConfinerError> {
    let mut env = BTreeMap::new();
    env.insert("ARLEN_AI_ENGINE_SOCKET".to_string(), SANDBOX_CONTRACT_SOCKET.to_string());
    env.insert("ARLEN_AI_PROXY_SOCKET".to_string(), SANDBOX_PROXY_SOCKET.to_string());
    env.insert("HOME".to_string(), require_abs(&paths.pi_state)?);
    if let Some(path) = token_file {
        env.insert("ARLEN_AI_ENGINE_TOKEN_FILE".to_string(), require_abs(path)?);
    }

    // /usr read-only gives node its shared libraries; the pi state dir is the
    // app's writable dir; no network at all. pi's only channel out of the sandbox
    // is the CONTRACT socket to this daemon (the plugins' Authorize/Report/Execute
    // round trip); the daemon, not pi, makes the model call.
    let skeleton = app_runtime_profile(
        Path::new("/usr"),
        &[Path::new(&paths.pi_state)],
        &[],
        env,
        NetworkPolicy::None,
    )?;

    // The Arlen extension's directory (read-only): pi's `--extension` entry plus
    // its sibling compiled modules, which `index.js` imports relatively.
    let ext_dir = require_abs(&paths.arlen_extension)?;
    let ext_dir = Path::new(&ext_dir)
        .parent()
        .and_then(|p| p.to_str())
        .ok_or(ConfinerError::RelativePath(paths.arlen_extension.clone()))?
        .to_string();

    // Plumbing: the node runtime + pi install + the Arlen extension dir read-only
    // (each at its host path so node resolves its own libs + the pi dist + the
    // extension's sibling modules), the two sockets read-write at their fixed
    // sandbox paths.
    let mut plumbing = vec![
        Bind::ReadOnly(require_abs(&paths.node_runtime)?, require_abs(&paths.node_runtime)?),
        Bind::ReadOnly(require_abs(&paths.pi_install)?, require_abs(&paths.pi_install)?),
        Bind::ReadOnly(ext_dir.clone(), ext_dir),
        Bind::ReadWrite(require_abs(&paths.contract_socket)?, SANDBOX_CONTRACT_SOCKET.to_string()),
    ];
    // The ai-proxy socket is bound ONLY when it exists. Today it never does: the
    // proxy serves D-Bus (`org.arlen.AIProxy1`) and binds no Unix socket, and
    // nothing in the sandbox reads `ARLEN_AI_PROXY_SOCKET` - pi's plugins talk to
    // the daemon over the CONTRACT socket, and the daemon makes the model call
    // itself through `ProxiedProvider`. An unconditional bind is therefore not a
    // stricter boundary, it is a fatal one: bwrap refuses to start when a bind
    // SOURCE is missing, so this killed the sidecar on every spawn
    // ("bwrap: Can't find source path .../ai-proxy.sock") and pi never ran in the
    // image at all. Kept conditional rather than deleted so a future proxy that
    // does bind a socket is plumbed in without re-deriving this.
    let proxy_socket = require_abs(&paths.proxy_socket)?;
    if Path::new(&proxy_socket).exists() {
        plumbing.push(Bind::ReadWrite(proxy_socket, SANDBOX_PROXY_SOCKET.to_string()));
    }
    // node's ELF interpreter is /lib64/ld-linux-x86-64.so.2 (a symlink resolving
    // into /usr on the host). /usr is bound, but the kernel resolves the
    // interpreter via the /lib64 (or /lib) path, which the sandbox root otherwise
    // lacks - so exec of node fails ENOENT without this. Bind the loader symlink
    // dirs when present (bwrap resolves the source symlink), the same loader-bind
    // the decoder workers' confinement needs.
    for loader in ["/lib64", "/lib"] {
        if Path::new(loader).exists() {
            plumbing.push(Bind::ReadOnly(loader.to_string(), loader.to_string()));
        }
    }
    Ok(skeleton.complete(plumbing, vec![]))
}

/// The full confined spawn argv: the bwrap flags then `-- <node> <pi_cli> --mode
/// rpc`. Pure (no spawn). `--mode rpc` runs pi as the RPC-over-stdio sidecar
/// (the `profile:rpc` entry); the `run_once` spawn adds only the stdio wiring and
/// the secret session-token env on top of this.
pub fn pi_sidecar_argv(confinement: &Confinement, paths: &SidecarPaths, system_prompt: &str) -> Vec<String> {
    let mut argv = confinement.bwrap_args();
    argv.push("--".to_string());
    argv.push(paths.node_bin.clone());
    argv.push(paths.pi_cli.clone());
    argv.push("--mode".to_string());
    argv.push("rpc".to_string());
    // Load the Arlen security extension (the gate + audit shims) into pi.
    argv.push("--extension".to_string());
    argv.push(paths.arlen_extension.clone());
    // The session system prompt the daemon composed (SessionInit's behaviour-inject
    // half) is delivered at spawn: SessionInit is daemon-driven, not an engine-
    // fetchable contract call, so the prompt reaches pi here, not over the socket.
    // `--system-prompt` replaces pi's default (context files + skills still append);
    // an empty prompt leaves the default, so a session with no composed prompt is a
    // clean no-op (the orchestrator that composes a non-empty one lands later).
    if !system_prompt.is_empty() {
        argv.push("--system-prompt".to_string());
        argv.push(system_prompt.to_string());
    }
    argv
}

/// Removes the per-run token file on drop, so the secret never outlives the
/// engine run even on an error/panic path.
struct TokenFileGuard {
    path: PathBuf,
}

impl Drop for TokenFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A random hex nonce for a per-run token filename.
fn token_nonce() -> std::io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Write the per-run session token to a fresh `0600` file in the (0700) state
/// dir and return a guard that removes it on drop. The directory + file modes
/// keep the secret owner-only.
///
/// The filename carries a per-run RANDOM NONCE: concurrent pi runs - the curator's
/// ephemeral runs AND the persistent supervisor - all share this one state dir, so
/// a fixed filename would let one run's write/delete corrupt another's
/// authentication and let a stray ephemeral run yank the live interactive
/// session's token. A unique name means each run owns its own file, so
/// `create_new` is safe (there is no stale file to race) and the guard removes
/// exactly this run's file.
fn write_token_file(state_dir: &str, token: &str) -> std::io::Result<(PathBuf, TokenFileGuard)> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(state_dir)?;
    let nonce = token_nonce()?;
    let path = Path::new(state_dir).join(format!("{SESSION_TOKEN_FILENAME}-{nonce}"));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    use std::io::Write;
    f.write_all(token.as_bytes())?;
    f.flush()?;
    Ok((path.clone(), TokenFileGuard { path }))
}

/// Parse bwrap's `--info-fd` JSON for `child-pid` (the sandboxed process's pid in
/// the daemon's namespace). bwrap may write more than one JSON object; the first
/// carrying `child-pid` wins. Returns None if absent/unparseable (fail-closed:
/// the caller treats a missing pid as a spawn fault).
fn parse_child_pid(info: &str) -> Option<u32> {
    // bwrap writes a single PRETTY-PRINTED (multi-line) JSON object to --info-fd
    // (and may emit more than one), so parse the whole content as a stream of JSON
    // values rather than line-by-line (a multi-line object is not valid JSON on any
    // single line). The first value carrying `child-pid` wins.
    serde_json::Deserializer::from_str(info)
        .into_iter::<serde_json::Value>()
        .flatten()
        .find_map(|v| v.get("child-pid").and_then(|p| p.as_u64()).and_then(|p| u32::try_from(p).ok()))
}

/// The real engine sidecar: spawns `pi --mode rpc` under the confinement built
/// here. Implements [`SpawnEngine`] so the supervisor's restart loop drives it.
pub struct PiSidecar {
    paths: SidecarPaths,
}

impl PiSidecar {
    /// Build the sidecar over resolved [`SidecarPaths`].
    pub fn new(paths: SidecarPaths) -> Self {
        Self { paths }
    }

    /// Spawn the confined engine and run it to exit. Separated from `run_once`
    /// so the fallible setup maps uniformly to [`EngineExit::Crashed`].
    async fn spawn_and_wait(
        &self,
        session_token: &str,
        system_prompt: &str,
        on_spawned: &(dyn Fn(u32) + Send + Sync),
        drive: Option<&tokio::net::UnixListener>,
    ) -> std::io::Result<EngineExit> {
        // The token reaches pi via a 0600 file in the rw state dir, not the argv.
        let (token_path, _guard) = write_token_file(&self.paths.pi_state, session_token)?;
        let token_path = token_path.to_string_lossy().into_owned();

        let confinement = pi_sidecar_confinement(&self.paths, Some(&token_path))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
        let mut argv = pi_sidecar_argv(&confinement, &self.paths, system_prompt);

        // A pipe whose write end bwrap inherits and writes its JSON info to (the
        // child-pid we bind the session to); the read end stays in the parent.
        // O_CLOEXEC on both, then cleared on the write end so it survives the
        // child's exec.
        let mut fds = [0i32; 2];
        // SAFETY: pipe2 fills the two-element array; the return is checked.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);
        // SAFETY: clear O_CLOEXEC on the write end so bwrap inherits it.
        if unsafe { libc::fcntl(write_fd, libc::F_SETFD, 0) } != 0 {
            let e = std::io::Error::last_os_error();
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            return Err(e);
        }
        // Splice `--info-fd <write_fd>` into the bwrap flags (before the `--`).
        let sep = argv.iter().position(|s| s == "--").unwrap_or(argv.len());
        argv.splice(sep..sep, ["--info-fd".to_string(), write_fd.to_string()]);

        // stdin/stdout are pi's RPC-over-stdio channel (held by the child; the
        // shell-facing proxy that drives them is a later wiring); stderr inherits
        // so pi's diagnostics reach the daemon log.
        let mut cmd = Command::new("bwrap");
        cmd.args(&argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            // Kill the confined pi tree if this `run_once` future is dropped
            // (e.g. an ephemeral autonomous run hitting its wall-clock timeout, or
            // the supervising task being aborted). Without this the dropped Child
            // is reaped but never killed, so a timed-out run would leak the whole
            // bwrap+node+pi tree (CPU/RAM/model-token drain). bwrap's
            // `--die-with-parent` only fires on DAEMON death, not future-drop.
            .kill_on_drop(true);
        let spawn_result = cmd.spawn();
        // Close the parent's copy of the write end unconditionally, so the read
        // end sees EOF once bwrap closes its copy (and so a spawn failure does
        // not leak it).
        // SAFETY: write_fd is owned here and not used after this.
        unsafe {
            libc::close(write_fd);
        }
        let mut child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                // SAFETY: read_fd is still owned; close it before returning.
                unsafe {
                    libc::close(read_fd);
                }
                return Err(e);
            }
        };

        // Read the info pipe to EOF (tiny, written promptly before exec) off the
        // async runtime, then parse the child pid.
        // SAFETY: read_fd is owned and not used elsewhere; the File takes it over.
        let info = tokio::task::spawn_blocking(move || {
            let mut f = unsafe { std::fs::File::from_raw_fd(read_fd) };
            let mut s = String::new();
            let _ = f.read_to_string(&mut s);
            s
        })
        .await
        .unwrap_or_default();

        match parse_child_pid(&info) {
            Some(pid) => on_spawned(pid),
            None => {
                warn!("bwrap did not report a child-pid on --info-fd; killing the sidecar");
                let _ = child.kill().await;
                return Ok(EngineExit::Crashed);
            }
        }

        // Phase-2-A drive channel: when a drive socket is provided, relay this
        // pi instance's RPC stdio against shell connections for the instance's
        // lifetime. pi's stdin/stdout were piped at spawn; take them now.
        // `serve_drive` loops, serving reconnecting shells (pi's stdin stays open
        // across a shell disconnect); it returns only if the listener breaks, in
        // which case the future idles. Either way a shell disconnect does NOT end
        // the engine - only pi's own exit (child.wait) does. With no drive socket
        // the engine runs headless exactly as before.
        // Take pi's piped stdio before the select so it does not borrow `child`
        // (which `child.wait()` needs mutably).
        let pi_stdin = child.stdin.take();
        let pi_stdout = child.stdout.take();
        let drive_fut = async {
            if let (Some(listener), Some(stdin), Some(stdout)) = (drive, pi_stdin, pi_stdout) {
                if let Err(e) = crate::rpc_proxy::serve_drive(listener, stdin, stdout).await {
                    warn!(error = %e, "pi drive session ended with an error");
                }
            }
            // The drive session is over (or never started); wait for pi to exit
            // rather than ending the run.
            std::future::pending::<()>().await
        };

        let status = tokio::select! {
            status = child.wait() => status?,
            _ = drive_fut => unreachable!("drive_fut never resolves"),
        };
        Ok(if status.success() { EngineExit::Clean } else { EngineExit::Crashed })
    }
}

#[async_trait]
impl SpawnEngine for PiSidecar {
    async fn run_once(
        &self,
        session_token: &str,
        system_prompt: &str,
        on_spawned: &(dyn Fn(u32) + Send + Sync),
        drive: Option<&tokio::net::UnixListener>,
    ) -> EngineExit {
        match self.spawn_and_wait(session_token, system_prompt, on_spawned, drive).await {
            Ok(exit) => exit,
            Err(e) => {
                error!(error = %e, "failed to spawn the confined pi sidecar");
                EngineExit::Crashed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> SidecarPaths {
        SidecarPaths {
            node_runtime: "/opt/arlen-node22".to_string(),
            node_bin: "/opt/arlen-node22/bin/node".to_string(),
            pi_install: "/opt/pi".to_string(),
            pi_cli: "/opt/pi/packages/coding-agent/dist/cli.js".to_string(),
            pi_state: "/home/u/.local/state/arlen/pi".to_string(),
            contract_socket: "/run/user/1000/arlen/ai-engine.sock".to_string(),
            proxy_socket: "/run/user/1000/arlen/ai-proxy.sock".to_string(),
            arlen_extension: "/opt/arlen/pi-plugins/dist/index.js".to_string(),
        }
    }

    fn args() -> Vec<String> {
        let conf = pi_sidecar_confinement(&paths(), None).expect("confinement");
        pi_sidecar_argv(&conf, &paths(), "")
    }

    #[test]
    fn the_system_prompt_is_appended_only_when_non_empty() {
        let conf = pi_sidecar_confinement(&paths(), None).expect("confinement");

        // Empty (the default session, no composed prompt): pi keeps its default,
        // so no --system-prompt flag is added.
        let bare = pi_sidecar_argv(&conf, &paths(), "");
        assert!(!bare.iter().any(|a| a == "--system-prompt"), "empty prompt adds no flag");

        // A composed session prompt is delivered at spawn as --system-prompt <text>,
        // after the program separator (so it is a pi arg, not a bwrap flag).
        let with = pi_sidecar_argv(&conf, &paths(), "You are Arlen's tidy-downloads behaviour.");
        let flag = with.iter().position(|a| a == "--system-prompt").expect("flag present");
        let sep = with.iter().position(|a| a == "--").expect("separator present");
        assert!(flag > sep, "--system-prompt is a pi arg (after `--`), not a bwrap flag");
        assert_eq!(with.get(flag + 1).map(String::as_str), Some("You are Arlen's tidy-downloads behaviour."));
    }

    /// The value bound (`--ro-bind`/`--bind`/`--setenv`) for a flag at a given
    /// destination, scanning the deterministic argv.
    fn dest_of(argv: &[String], flag: &str, src: &str) -> Option<String> {
        argv.windows(3)
            .find(|w| w[0] == flag && w[1] == src)
            .map(|w| w[2].clone())
    }

    #[test]
    fn the_sidecar_has_no_network() {
        // pi's only egress is the ai-proxy Unix socket bound in; the sandbox gets
        // its own (empty) net namespace.
        assert!(args().contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn the_contract_socket_is_bound_read_write() {
        let a = args();
        assert_eq!(
            dest_of(&a, "--bind", "/run/user/1000/arlen/ai-engine.sock"),
            Some(SANDBOX_CONTRACT_SOCKET.to_string()),
        );
    }

    /// bwrap refuses to start when a bind SOURCE is missing, so binding the
    /// ai-proxy socket unconditionally killed the sidecar on every spawn - the
    /// proxy serves D-Bus and binds no socket, so the path never exists. The bind
    /// must therefore appear only when the source is really there. `args()` uses
    /// fixture paths under /run/user/1000 that do not exist in the test
    /// environment, which is exactly the absent case.
    #[test]
    fn the_proxy_socket_is_bound_only_when_it_exists() {
        let a = args();
        assert_eq!(
            dest_of(&a, "--bind", "/run/user/1000/arlen/ai-proxy.sock"),
            None,
            "an absent proxy socket must not be bound: bwrap would refuse to start"
        );

        // And when it does exist, it IS bound at the fixed sandbox path.
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("ai-proxy.sock");
        std::fs::write(&sock, b"").expect("create the stand-in socket path");
        let mut paths = paths();
        paths.proxy_socket = sock.to_string_lossy().into_owned();
        let c = pi_sidecar_confinement(&paths, None).expect("confinement");
        assert_eq!(
            dest_of(&c.bwrap_args(), "--bind", &paths.proxy_socket),
            Some(SANDBOX_PROXY_SOCKET.to_string()),
        );
    }

    #[test]
    fn the_node_runtime_and_pi_install_are_read_only() {
        let a = args();
        assert_eq!(dest_of(&a, "--ro-bind", "/opt/arlen-node22"), Some("/opt/arlen-node22".to_string()));
        assert_eq!(dest_of(&a, "--ro-bind", "/opt/pi"), Some("/opt/pi".to_string()));
        // /usr is the read-only base for node's shared libraries.
        assert_eq!(dest_of(&a, "--ro-bind", "/usr"), Some("/usr".to_string()));
        // The Arlen extension's DIR (the parent of its entry) is read-only bound.
        assert_eq!(
            dest_of(&a, "--ro-bind", "/opt/arlen/pi-plugins/dist"),
            Some("/opt/arlen/pi-plugins/dist".to_string()),
        );
    }

    #[test]
    fn home_is_pinned_to_the_state_dir_not_the_host_home() {
        assert_eq!(dest_of(&args(), "--setenv", "HOME"), Some("/home/u/.local/state/arlen/pi".to_string()));
        // The plugins read the in-sandbox socket path, not the host one.
        assert_eq!(dest_of(&args(), "--setenv", "ARLEN_AI_ENGINE_SOCKET"), Some(SANDBOX_CONTRACT_SOCKET.to_string()));
    }

    #[test]
    fn the_program_tail_runs_pi_in_rpc_mode_over_node() {
        let a = args();
        let sep = a.iter().position(|s| s == "--").expect("a -- separator");
        assert_eq!(
            &a[sep + 1..],
            &[
                "/opt/arlen-node22/bin/node".to_string(),
                "/opt/pi/packages/coding-agent/dist/cli.js".to_string(),
                "--mode".to_string(),
                "rpc".to_string(),
                "--extension".to_string(),
                "/opt/arlen/pi-plugins/dist/index.js".to_string(),
            ],
        );
    }

    #[test]
    fn the_session_token_never_appears_in_the_argv() {
        // The token is the run_once spawn's secret (passed out of band, not in
        // the argv where a same-uid /proc reader would see it). Nothing in the
        // pure builder mentions a token at all.
        let a = args();
        assert!(
            !a.iter().any(|s| s.to_ascii_uppercase().contains("TOKEN")),
            "no token env or value is in the confined argv"
        );
    }

    #[test]
    fn a_relative_path_is_rejected() {
        let mut p = paths();
        p.pi_install = "opt/pi".to_string(); // not absolute
        assert!(matches!(pi_sidecar_confinement(&p, None), Err(ConfinerError::RelativePath(_))));
    }

    #[test]
    fn the_token_file_path_is_set_only_when_provided() {
        // None: token-free shape, no token-file env at all.
        let none = pi_sidecar_confinement(&paths(), None).expect("confinement");
        let none_argv = pi_sidecar_argv(&none, &paths(), "");
        assert_eq!(dest_of(&none_argv, "--setenv", "ARLEN_AI_ENGINE_TOKEN_FILE"), None);
        // Some: the file PATH (not the secret) is set so the plugins find the token.
        let tf = "/home/u/.local/state/arlen/pi/.arlen-session-token";
        let some = pi_sidecar_confinement(&paths(), Some(tf)).expect("confinement");
        let some_argv = pi_sidecar_argv(&some, &paths(), "");
        assert_eq!(
            dest_of(&some_argv, "--setenv", "ARLEN_AI_ENGINE_TOKEN_FILE"),
            Some(tf.to_string()),
        );
    }

    #[test]
    fn a_relative_token_file_path_is_rejected() {
        assert!(matches!(
            pi_sidecar_confinement(&paths(), Some("relative/token")),
            Err(ConfinerError::RelativePath(_)),
        ));
    }

    /// A full environment for the resolver, as a closure (no process-env mutation).
    fn full_env(key: &str) -> Option<String> {
        match key {
            "ARLEN_PI_NODE_RUNTIME" => Some("/opt/arlen-node22".to_string()),
            "ARLEN_PI_INSTALL" => Some("/opt/pi".to_string()),
            "ARLEN_PI_EXTENSION" => Some("/opt/arlen/pi-plugins/dist/index.js".to_string()),
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".to_string()),
            "XDG_STATE_HOME" => Some("/home/u/.local/state".to_string()),
            _ => None,
        }
    }

    #[test]
    fn resolve_derives_node_pi_state_and_proxy_paths() {
        let p = SidecarPaths::resolve(full_env, "/run/user/1000/arlen/ai-engine.sock".to_string())
            .expect("resolve");
        assert_eq!(p.node_bin, "/opt/arlen-node22/bin/node");
        assert_eq!(p.pi_cli, "/opt/pi/packages/coding-agent/dist/cli.js");
        assert_eq!(p.pi_state, "/home/u/.local/state/arlen/pi");
        assert_eq!(p.proxy_socket, "/run/user/1000/arlen/ai-proxy.sock");
        assert_eq!(p.contract_socket, "/run/user/1000/arlen/ai-engine.sock");
        assert_eq!(p.arlen_extension, "/opt/arlen/pi-plugins/dist/index.js");
        // The resolved paths build a valid confinement.
        assert!(pi_sidecar_confinement(&p, None).is_ok());
    }

    #[test]
    fn resolve_falls_back_to_home_for_the_state_dir() {
        let env = |k: &str| match k {
            "XDG_STATE_HOME" => None,
            "HOME" => Some("/home/u".to_string()),
            other => full_env(other),
        };
        let p = SidecarPaths::resolve(env, "/run/user/1000/arlen/ai-engine.sock".to_string())
            .expect("resolve");
        assert_eq!(p.pi_state, "/home/u/.local/state/arlen/pi");
    }

    #[test]
    fn resolve_fails_closed_without_the_node_runtime() {
        let env = |k: &str| if k == "ARLEN_PI_NODE_RUNTIME" { None } else { full_env(k) };
        let err = SidecarPaths::resolve(env, "/run/x/ai-engine.sock".to_string()).unwrap_err();
        assert!(err.contains("ARLEN_PI_NODE_RUNTIME"), "names the missing var: {err}");
    }

    #[test]
    fn resolve_fails_closed_without_the_pi_install() {
        let env = |k: &str| if k == "ARLEN_PI_INSTALL" { None } else { full_env(k) };
        assert!(SidecarPaths::resolve(env, "/run/x/ai-engine.sock".to_string())
            .unwrap_err()
            .contains("ARLEN_PI_INSTALL"));
    }

    #[test]
    fn resolve_rejects_a_non_absolute_required_path() {
        let env = |k: &str| if k == "ARLEN_PI_INSTALL" { Some("opt/pi".to_string()) } else { full_env(k) };
        // A relative install dir makes the derived pi-cli path non-absolute.
        assert!(SidecarPaths::resolve(env, "/run/x/ai-engine.sock".to_string())
            .unwrap_err()
            .contains("not absolute"));
    }

    #[test]
    fn parse_child_pid_reads_the_bwrap_info() {
        assert_eq!(parse_child_pid(r#"{"child-pid": 4242, "cgroup": "x"}"#), Some(4242));
        // bwrap may emit more than one object; the one carrying child-pid wins.
        assert_eq!(parse_child_pid("{\"exit-code\":0}\n{\"child-pid\":7}"), Some(7));
        assert_eq!(parse_child_pid("{}"), None);
        assert_eq!(parse_child_pid("not json"), None);
        assert_eq!(parse_child_pid(""), None);
    }

    #[test]
    fn the_token_file_is_written_0600_and_removed_on_drop() {
        use std::os::unix::fs::MetadataExt;
        // A unique temp state dir (no tempfile dev-dep; uniqueness via pid).
        let dir = std::env::temp_dir().join(format!("arlen-pi-tok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.to_string_lossy().into_owned();

        let token_path;
        {
            let (path, _guard) = write_token_file(&dir_s, "s3cr3t-token").expect("write token");
            token_path = path.clone();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "s3cr3t-token");
            // Owner-only (0600).
            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        }
        // The guard removed it on drop, so the secret does not outlive the run.
        assert!(!token_path.exists(), "the token file is removed when the guard drops");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_child_pid_handles_bwraps_multi_line_info() {
        // bwrap writes ONE pretty-printed multi-line JSON object (with many
        // namespace keys from --unshare-*), so the child-pid must come from parsing
        // the whole content as a JSON stream, not line-by-line (no single line is
        // valid JSON). This is the real shape `--info-fd` produces under the pi
        // confinement; a regression here means the daemon cannot learn pi's pid.
        let info = "{\n    \"child-pid\": 4242,\n    \"cgroup-namespace\": 1,\n    \"ipc-namespace\": 2,\n    \"mnt-namespace\": 3,\n    \"net-namespace\": 4,\n    \"pid-namespace\": 5,\n    \"uts-namespace\": 6\n}\n";
        assert_eq!(parse_child_pid(info), Some(4242));
        // No child-pid, or empty input -> None (fail-closed: a missing pid is a fault).
        assert_eq!(parse_child_pid("{\n    \"mnt-namespace\": 3\n}\n"), None);
        assert_eq!(parse_child_pid(""), None);
    }

    /// On-host (needs a userns-capable host + a real node at ARLEN_PI_NODE_RUNTIME
    /// and a pi install at ARLEN_PI_INSTALL): the confinement actually starts node
    /// and bwrap reports a child-pid. Validates the base bind set (node's dynamic
    /// loader) + the --info-fd pid parsing end-to-end against a real process,
    /// running `node --version` as the trivial confined payload. `#[ignore]d` like
    /// the other host-gated spawn tests.
    #[tokio::test]
    #[ignore = "needs a userns-capable host + a real node runtime (ARLEN_PI_NODE_RUNTIME) + pi install (ARLEN_PI_INSTALL/EXTENSION)"]
    async fn the_confined_node_starts_and_reports_a_child_pid() {
        // Self-contained: the confinement binds the pi-state dir + the contract and
        // ai-proxy sockets, and bwrap needs each bind SOURCE to exist. Point the
        // state/runtime dirs at a temp tree and dummy-bind both sockets, so the test
        // needs only a real node + pi install (the ARLEN_PI_* env), not a live daemon
        // or ai-proxy.
        use std::os::unix::net::UnixListener;
        let tmp = std::env::temp_dir().join(format!("arlen-pi-spawn-{}", std::process::id()));
        let arlen = tmp.join("arlen");
        std::fs::create_dir_all(arlen.join("pi")).expect("temp state dir");
        let contract = arlen.join("ai-engine.sock");
        let proxy = arlen.join("ai-proxy.sock");
        let _contract_l = UnixListener::bind(&contract).expect("dummy contract socket");
        let _proxy_l = UnixListener::bind(&proxy).expect("dummy ai-proxy socket");

        let tmp_s = tmp.to_string_lossy().into_owned();
        let p = SidecarPaths::resolve(
            // pi_state = {XDG_STATE_HOME}/arlen/pi; proxy = {XDG_RUNTIME_DIR}/arlen/
            // ai-proxy.sock - point both at the temp tree; node/pi/extension stay real.
            move |k| match k {
                "XDG_STATE_HOME" | "XDG_RUNTIME_DIR" => Some(tmp_s.clone()),
                _ => std::env::var(k).ok(),
            },
            contract.to_string_lossy().into_owned(),
        )
        .expect("resolve sidecar paths from the env");
        let conf = pi_sidecar_confinement(&p, None).expect("confinement");
        // The bwrap argv with the program tail swapped for `node --version` (a
        // trivial confined payload that exits 0), the confinement kept intact.
        let mut argv = conf.bwrap_args();
        argv.push("--".to_string());
        argv.push(p.node_bin.clone());
        argv.push("--version".to_string());

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        let (read_fd, write_fd) = (fds[0], fds[1]);
        assert_eq!(unsafe { libc::fcntl(write_fd, libc::F_SETFD, 0) }, 0);
        let sep = argv.iter().position(|s| s == "--").unwrap();
        argv.splice(sep..sep, ["--info-fd".to_string(), write_fd.to_string()]);

        let mut child = Command::new("bwrap")
            .args(&argv)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn bwrap");
        unsafe { libc::close(write_fd) };
        let info = tokio::task::spawn_blocking(move || {
            let mut f = unsafe { std::fs::File::from_raw_fd(read_fd) };
            let mut s = String::new();
            let _ = f.read_to_string(&mut s);
            s
        })
        .await
        .unwrap();

        let pid = parse_child_pid(&info).expect("bwrap reported a child-pid");
        assert!(pid > 1, "the child-pid is a real pid: {pid}");
        let status = child.wait().await.expect("wait");
        assert!(status.success(), "node --version runs cleanly under the confinement");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
