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
//! network (pi reaches the model only over the ai-proxy Unix socket, bound in;
//! all real egress is the proxy's), a writable tmpfs `/tmp` + the pi state dir,
//! and the contract + ai-proxy sockets bound read-write at fixed in-sandbox
//! paths the plugins connect to. The exact base bind set (node also needs its
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
    // app's writable dir; no network (pi's only egress is the proxy socket).
    let skeleton = app_runtime_profile(
        Path::new("/usr"),
        &[Path::new(&paths.pi_state)],
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
    let plumbing = vec![
        Bind::ReadOnly(require_abs(&paths.node_runtime)?, require_abs(&paths.node_runtime)?),
        Bind::ReadOnly(require_abs(&paths.pi_install)?, require_abs(&paths.pi_install)?),
        Bind::ReadOnly(ext_dir.clone(), ext_dir),
        Bind::ReadWrite(require_abs(&paths.contract_socket)?, SANDBOX_CONTRACT_SOCKET.to_string()),
        Bind::ReadWrite(require_abs(&paths.proxy_socket)?, SANDBOX_PROXY_SOCKET.to_string()),
    ];
    Ok(skeleton.complete(plumbing, vec![]))
}

/// The full confined spawn argv: the bwrap flags then `-- <node> <pi_cli> --mode
/// rpc`. Pure (no spawn). `--mode rpc` runs pi as the RPC-over-stdio sidecar
/// (the `profile:rpc` entry); the `run_once` spawn adds only the stdio wiring and
/// the secret session-token env on top of this.
pub fn pi_sidecar_argv(confinement: &Confinement, paths: &SidecarPaths) -> Vec<String> {
    let mut argv = confinement.bwrap_args();
    argv.push("--".to_string());
    argv.push(paths.node_bin.clone());
    argv.push(paths.pi_cli.clone());
    argv.push("--mode".to_string());
    argv.push("rpc".to_string());
    // Load the Arlen security extension (the gate + audit shims) into pi.
    argv.push("--extension".to_string());
    argv.push(paths.arlen_extension.clone());
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

/// Write the per-run session token to a fresh `0600` file in the (0700) state
/// dir and return a guard that removes it on drop. The directory + file modes
/// keep the secret owner-only; `create_new` would race a stale file, so an
/// existing one is truncated by `OpenOptions::write+truncate` after the mode is
/// fixed.
fn write_token_file(state_dir: &str, token: &str) -> std::io::Result<(PathBuf, TokenFileGuard)> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(state_dir)?;
    let path = Path::new(state_dir).join(SESSION_TOKEN_FILENAME);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
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
    for line in info.split('\n').filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(pid) = v.get("child-pid").and_then(|p| p.as_u64()) {
                return u32::try_from(pid).ok();
            }
        }
    }
    None
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
        on_spawned: &(dyn Fn(u32) + Send + Sync),
    ) -> std::io::Result<EngineExit> {
        // The token reaches pi via a 0600 file in the rw state dir, not the argv.
        let (token_path, _guard) = write_token_file(&self.paths.pi_state, session_token)?;
        let token_path = token_path.to_string_lossy().into_owned();

        let confinement = pi_sidecar_confinement(&self.paths, Some(&token_path))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
        let mut argv = pi_sidecar_argv(&confinement, &self.paths);

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
            .stderr(Stdio::inherit());
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

        let status = child.wait().await?;
        Ok(if status.success() { EngineExit::Clean } else { EngineExit::Crashed })
    }
}

#[async_trait]
impl SpawnEngine for PiSidecar {
    async fn run_once(
        &self,
        session_token: &str,
        on_spawned: &(dyn Fn(u32) + Send + Sync),
    ) -> EngineExit {
        match self.spawn_and_wait(session_token, on_spawned).await {
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
        pi_sidecar_argv(&conf, &paths())
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
    fn the_contract_and_proxy_sockets_are_bound_read_write() {
        let a = args();
        assert_eq!(
            dest_of(&a, "--bind", "/run/user/1000/arlen/ai-engine.sock"),
            Some(SANDBOX_CONTRACT_SOCKET.to_string()),
        );
        assert_eq!(
            dest_of(&a, "--bind", "/run/user/1000/arlen/ai-proxy.sock"),
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
        let none_argv = pi_sidecar_argv(&none, &paths());
        assert_eq!(dest_of(&none_argv, "--setenv", "ARLEN_AI_ENGINE_TOKEN_FILE"), None);
        // Some: the file PATH (not the secret) is set so the plugins find the token.
        let tf = "/home/u/.local/state/arlen/pi/.arlen-session-token";
        let some = pi_sidecar_confinement(&paths(), Some(tf)).expect("confinement");
        let some_argv = pi_sidecar_argv(&some, &paths());
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

    /// On-host (needs a userns-capable host + a real node at ARLEN_PI_NODE_RUNTIME
    /// and a pi install at ARLEN_PI_INSTALL): the confinement actually starts node
    /// and bwrap reports a child-pid. Validates the base bind set (node's dynamic
    /// loader) + the --info-fd pid parsing end-to-end against a real process,
    /// running `node --version` as the trivial confined payload. `#[ignore]d` like
    /// the other host-gated spawn tests.
    #[tokio::test]
    #[ignore = "needs a userns-capable host + a real node runtime + pi install"]
    async fn the_confined_node_starts_and_reports_a_child_pid() {
        let p = SidecarPaths::resolve(
            |k| std::env::var(k).ok(),
            "/run/user/1000/arlen/ai-engine.sock".to_string(),
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
    }
}
