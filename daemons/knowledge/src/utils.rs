/// Escape a string for safe interpolation into a Cypher single-quoted literal.
///
/// Cypher uses `'...'` for string literals with `\` as the escape character.
/// This function escapes backslashes and single quotes so that user-supplied
/// values (file paths, app IDs, window titles) cannot break out of the string.
pub fn escape_cypher(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Resolve a daemon socket path per the standard Arlen 3-tier
/// convention: the `env_var` override (non-empty) wins, else
/// `$XDG_RUNTIME_DIR/arlen/<file_name>` (the per-user path, i.e.
/// `/run/user/{uid}/arlen/<file_name>`), else `/run/arlen/<file_name>`.
///
/// The knowledge daemon does not depend on `os-sdk` (it carries its own
/// raw Event Bus client), so the shared `os_sdk::runtime::socket_path`
/// resolver is reproduced here. The precedence must match it exactly:
/// the `ARLEN_*_SOCKET` env override stays tier 1 — it is the contract
/// the dev stack and the integration harness pin sockets through, and
/// the systemd unit pins `ARLEN_DAEMON_SOCKET` per-uid.
pub fn socket_path(env_var: &str, file_name: &str) -> String {
    let env_val = std::env::var(env_var).ok();
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    let path = resolve_socket(env_val.as_deref(), xdg.as_deref(), file_name);
    // Best-effort: ensure the per-user `arlen/` parent exists so the
    // daemon binds cleanly in a dev session. Skip when env-pinned (the
    // launcher owns that parent).
    if env_val.as_deref().filter(|s| !s.is_empty()).is_none() {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    path
}

/// Pure precedence backing [`socket_path`], split out so it can be
/// tested without mutating the process environment.
fn resolve_socket(env_val: Option<&str>, xdg: Option<&str>, file_name: &str) -> String {
    if let Some(p) = env_val.filter(|s| !s.is_empty()) {
        return p.to_string();
    }
    if let Some(dir) = xdg.filter(|s| !s.is_empty()) {
        return format!("{dir}/arlen/{file_name}");
    }
    format!("/run/arlen/{file_name}")
}

/// Pure precedence for a per-user DATA path (the graph + SQLite store),
/// backing the daemon's `pick_data_path` and testable without env. A
/// non-empty `pinned` env wins (the launcher / systemd-unit contract),
/// else `$XDG_DATA_HOME/arlen/<name>`, else `$HOME/.local/share/arlen/<name>`
/// (matching the unit's pinned `~/.local/share/arlen/...`), else the
/// system-wide `system_default` as a last resort. The per-user default
/// keeps two profile-uids from sharing one store even absent the env, the
/// same fail-safe the socket layer has (profile-system-plan.md PR-R1).
pub(crate) fn resolve_data_path(
    pinned: Option<&str>,
    xdg: Option<&str>,
    home: Option<&str>,
    name: &str,
    system_default: &str,
) -> String {
    if let Some(p) = pinned.filter(|s| !s.is_empty()) {
        return p.to_string();
    }
    if let Some(dir) = xdg.filter(|s| !s.is_empty()) {
        return format!("{dir}/arlen/{name}");
    }
    if let Some(h) = home.filter(|s| !s.is_empty()) {
        return format!("{h}/.local/share/arlen/{name}");
    }
    system_default.to_string()
}

/// The content-addressed merge identity of a relation fact: a length-delimited
/// SHA-256 over the content tuple `(from_label, from_id, rel, to_label, to_id)`,
/// rendered as lowercase hex (graph-drift.md §2 / GD-R1).
///
/// It is orthogonal to `op_id`. `op_id` is the per-device write idempotency key
/// (the agent derives it from its correlation id, so each device's write of the
/// same fact carries a different one); `merge_key` is identical on every device
/// that asserts the same content tuple, so a future cross-device union dedups
/// two writes of one fact to a single membership identity. The server-stamped
/// `valid_at` is deliberately NOT in the tuple: it differs per device by
/// construction, so including it would defeat the dedup. The provenance
/// (`origin`/`prov_beh`) is likewise excluded so the same fact asserted by
/// different paths (the agent write vs the promotion pipeline) or behaviours
/// still converges to one identity; trust between competing assertions is the
/// resolve pass's job (GD-R3), not the merge key's. The length prefix per part
/// means no concatenation of distinct tuples can collide (`("ab","c")` and
/// `("a","bc")` hash apart). The output is fixed 64-char hex, so it carries no
/// quote or backslash and is safe to interpolate into a Cypher literal.
pub(crate) fn content_merge_key(
    from_label: &str,
    from_id: &str,
    rel: &str,
    to_label: &str,
    to_id: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [from_label, from_id, rel, to_label, to_id] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod socket_path_tests {
    use super::resolve_socket;

    #[test]
    fn env_override_wins() {
        let p = resolve_socket(Some("/pinned.sock"), Some("/run/user/1000"), "knowledge.sock");
        assert_eq!(p, "/pinned.sock");
    }

    #[test]
    fn empty_env_falls_through_to_xdg() {
        let p = resolve_socket(Some(""), Some("/run/user/1000"), "knowledge.sock");
        assert_eq!(p, "/run/user/1000/arlen/knowledge.sock");
    }

    #[test]
    fn xdg_is_per_user() {
        let p = resolve_socket(None, Some("/run/user/1000"), "event-bus-consumer.sock");
        assert_eq!(p, "/run/user/1000/arlen/event-bus-consumer.sock");
    }

    #[test]
    fn run_arlen_last_resort() {
        let p = resolve_socket(None, None, "event-bus-producer.sock");
        assert_eq!(p, "/run/arlen/event-bus-producer.sock");
    }

    use super::resolve_data_path;

    #[test]
    fn data_env_override_wins() {
        let p = resolve_data_path(
            Some("/pinned/events.db"),
            Some("/home/u/.local/share"),
            Some("/home/u"),
            "events.db",
            "/var/lib/arlen/knowledge/events.db",
        );
        assert_eq!(p, "/pinned/events.db");
    }

    #[test]
    fn data_falls_through_to_xdg_then_home_then_system() {
        // XDG_DATA_HOME wins over HOME.
        assert_eq!(
            resolve_data_path(None, Some("/x/share"), Some("/home/u"), "graph", "/var/lib/arlen/knowledge/graph"),
            "/x/share/arlen/graph"
        );
        // No XDG: derive from HOME (matching the unit's ~/.local/share/arlen).
        assert_eq!(
            resolve_data_path(None, None, Some("/home/u"), "events.db", "/var/lib/arlen/knowledge/events.db"),
            "/home/u/.local/share/arlen/events.db"
        );
        // Neither: the system path is the last resort.
        assert_eq!(
            resolve_data_path(None, None, None, "graph", "/var/lib/arlen/knowledge/graph"),
            "/var/lib/arlen/knowledge/graph"
        );
        // An empty XDG falls through (not treated as a valid base).
        assert_eq!(
            resolve_data_path(None, Some(""), Some("/home/u"), "events.db", "/sys/events.db"),
            "/home/u/.local/share/arlen/events.db"
        );
    }
}

#[cfg(test)]
mod escape_cypher_tests {
    use super::escape_cypher;

    #[test]
    fn a_quote_is_escaped_so_a_literal_cannot_be_broken_out_of() {
        // A hostile path/app_id from a producer event lands inside a '...' Cypher
        // literal (promotion.rs); the quote must be escaped so it cannot terminate
        // the literal and inject.
        assert_eq!(escape_cypher("a'b"), "a\\'b");
        assert_eq!(
            escape_cypher("'; MATCH (n) DETACH DELETE n; //"),
            "\\'; MATCH (n) DETACH DELETE n; //"
        );
    }

    #[test]
    fn a_backslash_is_escaped_first_so_it_cannot_smuggle_an_escape() {
        // Backslash-first is the load-bearing order: a literal backslash becomes
        // `\\` before the quote pass, so `\'` in the input cannot become an
        // escaped-quote that swallows the closing quote, and a `\u`/`\n` cannot be
        // smuggled (the backslash is already doubled).
        assert_eq!(escape_cypher("a\\b"), "a\\\\b");
        assert_eq!(escape_cypher("\\'"), "\\\\\\'");
        assert_eq!(escape_cypher("\\u0041"), "\\\\u0041");
    }

    #[test]
    fn a_plain_path_is_unchanged() {
        assert_eq!(escape_cypher("/home/u/file.rs"), "/home/u/file.rs");
    }
}
