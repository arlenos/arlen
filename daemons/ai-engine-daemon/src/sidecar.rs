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

use arlen_confiner::{app_runtime_profile, Bind, Confinement, ConfinerError, NetworkPolicy};
use std::collections::BTreeMap;
use std::path::Path;

/// The fixed in-sandbox path the daemon's contract socket is bound to. The host
/// path varies with `XDG_RUNTIME_DIR`; the sandbox sees a stable path, and the
/// plugins read it from `ARLEN_AI_ENGINE_SOCKET`.
pub const SANDBOX_CONTRACT_SOCKET: &str = "/run/arlen/ai-engine.sock";
/// The fixed in-sandbox path the ai-proxy socket is bound to (pi's only egress).
pub const SANDBOX_PROXY_SOCKET: &str = "/run/arlen/ai-proxy.sock";

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
/// The session token is NOT set here: it is a secret the `run_once` spawn passes
/// to the child, kept out of this pure (loggable) builder AND out of the bwrap
/// argv (where it would be visible to a same-uid `/proc/<pid>/cmdline` reader).
pub fn pi_sidecar_confinement(paths: &SidecarPaths) -> Result<Confinement, ConfinerError> {
    let mut env = BTreeMap::new();
    env.insert("ARLEN_AI_ENGINE_SOCKET".to_string(), SANDBOX_CONTRACT_SOCKET.to_string());
    env.insert("ARLEN_AI_PROXY_SOCKET".to_string(), SANDBOX_PROXY_SOCKET.to_string());
    env.insert("HOME".to_string(), require_abs(&paths.pi_state)?);

    // /usr read-only gives node its shared libraries; the pi state dir is the
    // app's writable dir; no network (pi's only egress is the proxy socket).
    let skeleton = app_runtime_profile(
        Path::new("/usr"),
        &[Path::new(&paths.pi_state)],
        env,
        NetworkPolicy::None,
    )?;

    // Plumbing: the node runtime + pi install read-only (each at its host path so
    // node resolves its own libs + the pi dist), the two sockets read-write at
    // their fixed sandbox paths.
    let plumbing = vec![
        Bind::ReadOnly(require_abs(&paths.node_runtime)?, require_abs(&paths.node_runtime)?),
        Bind::ReadOnly(require_abs(&paths.pi_install)?, require_abs(&paths.pi_install)?),
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
    argv
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
        }
    }

    fn args() -> Vec<String> {
        let conf = pi_sidecar_confinement(&paths()).expect("confinement");
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
        assert!(matches!(pi_sidecar_confinement(&p), Err(ConfinerError::RelativePath(_))));
    }
}
