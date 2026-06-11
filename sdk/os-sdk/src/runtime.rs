//! Per-user daemon socket resolution.
//!
//! Every Arlen daemon socket follows one 3-tier convention so a
//! privileged per-profile launcher, the dev stack and the integration
//! harness can all pin sockets per profile-uid, while a daemon started
//! in a plain dev session still resolves somewhere deterministic. The
//! canonical per-user path is `$XDG_RUNTIME_DIR/arlen/<name>.sock`,
//! which on a logind session is `/run/user/{uid}/arlen/<name>.sock`,
//! the same shape `notification.sock`, `modulesd.sock` and the audit
//! sockets already use.

use std::path::PathBuf;

/// Resolve a per-user daemon socket path with the standard 3-tier
/// fallback:
///
/// 1. the `env_var` override (e.g. `ARLEN_DAEMON_SOCKET`) if set and
///    non-empty — the launcher's pinning contract, used by the dev
///    stack, the integration harness, and any privileged system
///    launcher;
/// 2. `$XDG_RUNTIME_DIR/arlen/<file_name>` — the normal per-user
///    session, which is `/run/user/{uid}/arlen/<file_name>`. The
///    `arlen/` parent dir is created if absent (best-effort), so a dev
///    daemon starts cleanly without a launcher pre-making it;
/// 3. `/run/arlen/<file_name>` — the system last resort for a
///    privileged launcher with no per-user runtime dir.
///
/// `env_var` is the override env *name* (not its value). `file_name`
/// is the bare socket file (e.g. `"knowledge.sock"`), never an
/// absolute path.
#[must_use]
pub fn socket_path(env_var: &str, file_name: &str) -> PathBuf {
    let env_val = std::env::var(env_var).ok();
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    let path = resolve(env_val.as_deref(), xdg.as_deref(), file_name);
    // Best-effort: ensure the per-user `arlen/` parent exists so a dev
    // daemon binds cleanly. Only meaningful on the XDG branch; harmless
    // for an env-pinned path (its parent is the launcher's concern) and
    // for `/run/arlen` (a privileged launcher owns it). Mirrors
    // `pick_daemon_socket`'s historical behaviour.
    if env_val.as_deref().filter(|s| !s.is_empty()).is_none() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    path
}

/// Pure precedence: env override (non-empty) wins, else
/// `$XDG_RUNTIME_DIR/arlen/<file_name>` (non-empty XDG), else
/// `/run/arlen/<file_name>`. Split out so the contract is unit-tested
/// without mutating the process environment (flaky under parallel test
/// runs).
fn resolve(env_val: Option<&str>, xdg: Option<&str>, file_name: &str) -> PathBuf {
    if let Some(p) = env_val.filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(dir) = xdg.filter(|s| !s.is_empty()) {
        return PathBuf::from(dir).join("arlen").join(file_name);
    }
    PathBuf::from("/run").join("arlen").join(file_name)
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use std::path::PathBuf;

    #[test]
    fn env_override_wins_over_everything() {
        let p = resolve(
            Some("/pinned/by/launcher.sock"),
            Some("/run/user/1000"),
            "knowledge.sock",
        );
        assert_eq!(p, PathBuf::from("/pinned/by/launcher.sock"));
    }

    #[test]
    fn empty_env_override_does_not_win() {
        // An exported-but-empty override must fall through to XDG, not
        // resolve to an empty path.
        let p = resolve(Some(""), Some("/run/user/1000"), "knowledge.sock");
        assert_eq!(p, PathBuf::from("/run/user/1000/arlen/knowledge.sock"));
    }

    #[test]
    fn xdg_fallback_is_per_user() {
        let p = resolve(None, Some("/run/user/1000"), "event-bus-producer.sock");
        assert_eq!(
            p,
            PathBuf::from("/run/user/1000/arlen/event-bus-producer.sock")
        );
    }

    #[test]
    fn empty_xdg_falls_to_run_arlen() {
        let p = resolve(None, Some(""), "knowledge.sock");
        assert_eq!(p, PathBuf::from("/run/arlen/knowledge.sock"));
    }

    #[test]
    fn run_arlen_is_the_last_resort() {
        let p = resolve(None, None, "knowledge.sock");
        assert_eq!(p, PathBuf::from("/run/arlen/knowledge.sock"));
    }
}
