/// `lunaris:host/graph` import implementation.
///
/// Graph access policy: every Cypher query is scoped to a namespace
/// that the daemon extracts at parse time and matches against the
/// module's `graph.read` / `graph.write` allowlists. Cross-namespace
/// queries are split into per-namespace fragments and fail closed if
/// any fragment is not allowed.
///
/// The wire-level Cypher round trip happens through `os-sdk`'s
/// `UnixGraphClient`. This module is the policy layer.

use crate::error::{DaemonError, Result};
use crate::host::CapabilityContext;

/// Coarse classifier for a Cypher statement. The daemon refuses
/// anything ambiguous: structured rejection beats letting a
/// permissive query slip through. A real query parser is overkill for
/// the shapes modules actually emit, but the simple classifier here is
/// only used for capability gating, not for correctness, so anything
/// it can not classify is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    Read,
    Write,
    Unknown,
}

pub fn classify(cypher: &str) -> QueryKind {
    let upper = cypher.to_uppercase();
    let has_write = ["CREATE", "MERGE", "DELETE", "SET", "REMOVE", "DROP"]
        .iter()
        .any(|kw| upper.contains(kw));
    let has_read = ["MATCH", "RETURN", "WITH"].iter().any(|kw| upper.contains(kw));
    match (has_write, has_read) {
        (true, _) => QueryKind::Write,
        (false, true) => QueryKind::Read,
        (false, false) => QueryKind::Unknown,
    }
}

/// Best-effort namespace extraction from a Cypher query. Returns the
/// *first* `:label` after a node-pattern paren, e.g. `(f:core.File)`
/// → `core.File`. Used by older callers; new code should use
/// [`extract_all_namespaces`] which catches every label referenced
/// by the query rather than just the first.
pub fn extract_namespace(cypher: &str) -> Option<String> {
    extract_all_namespaces(cypher).into_iter().next()
}

/// Extract every `:label` namespace the Cypher references. Walks the
/// string once, skipping content inside string literals (`"..."` and
/// `'...'`, with `\` escapes) and Cypher comments (`// to end of
/// line` and `/* ... */`).
///
/// Codex S6 critical finding fix: the previous single-namespace
/// extraction let an attacker get past the capability gate by
/// putting an allowed label first and then querying disallowed
/// namespaces later in the same Cypher string. The knowledge daemon
/// (Phase 1A) has no scope enforcement of its own, so the gate has
/// to cover the *whole* query, not just its prefix.
pub fn extract_all_namespaces(cypher: &str) -> Vec<String> {
    let bytes = cypher.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            // String literals: skip to the matching quote, honouring `\`
            // escapes. Cypher accepts both single and double quotes.
            b'"' | b'\'' => {
                let quote = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            // Line comment: skip to next newline.
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // Block comment: skip to closing `*/`.
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // Label / relationship-type marker. Walk forward over the
            // identifier characters and emit the namespace string.
            // `::` (Cypher type cast) collapses to one logical marker
            // by skipping the extra colon.
            b':' => {
                let mut start = i + 1;
                if start < bytes.len() && bytes[start] == b':' {
                    start += 1;
                }
                let mut end = start;
                while end < bytes.len() && is_label_char(bytes[end]) {
                    end += 1;
                }
                if end > start {
                    if let Ok(label) = std::str::from_utf8(&bytes[start..end]) {
                        if !label.is_empty() {
                            out.push(label.to_string());
                        }
                    }
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

fn is_label_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

/// Codex S6 critical finding fix: every namespace the Cypher
/// touches must pass the capability allowlist, not just the first.
/// `Unknown` query kinds and Cypher with no labels at all stay
/// rejected closed (no namespace → no policy basis).
pub fn check_query(ctx: &CapabilityContext, cypher: &str) -> Result<QueryKind> {
    let kind = classify(cypher);
    let namespaces = extract_all_namespaces(cypher);

    if namespaces.is_empty() {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: "graph.query (no namespace)".into(),
        });
    }

    for namespace in &namespaces {
        let allowed = match kind {
            QueryKind::Read => ctx.allow_graph_read(namespace),
            QueryKind::Write => ctx.allow_graph_write(namespace),
            QueryKind::Unknown => false,
        };
        if !allowed {
            return Err(DaemonError::CapabilityDenied {
                module_id: ctx.module_id.clone(),
                capability: format!(
                    "graph.{} ({namespace})",
                    match kind {
                        QueryKind::Read => "read",
                        QueryKind::Write => "write",
                        QueryKind::Unknown => "unknown",
                    },
                ),
            });
        }
    }
    Ok(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunaris_modules::{GraphCapability, ModuleCapabilities};

    fn ctx(read: Vec<&str>, write: Vec<&str>) -> CapabilityContext {
        let mut caps = ModuleCapabilities::default();
        caps.graph = Some(GraphCapability {
            read: read.into_iter().map(String::from).collect(),
            write: write.into_iter().map(String::from).collect(),
        });
        CapabilityContext::new("x", caps)
    }

    #[test]
    fn classify_read() {
        assert_eq!(classify("MATCH (f:core.File) RETURN f"), QueryKind::Read);
    }

    #[test]
    fn classify_write() {
        assert_eq!(classify("CREATE (f:core.File {name: 'x'})"), QueryKind::Write);
        assert_eq!(classify("MATCH (f:core.File) SET f.name = 'x'"), QueryKind::Write);
        assert_eq!(classify("MATCH (f:core.File) DELETE f"), QueryKind::Write);
    }

    #[test]
    fn classify_unknown_when_no_keywords() {
        assert_eq!(classify("EXPLAIN PROFILE"), QueryKind::Unknown);
    }

    #[test]
    fn namespace_extraction() {
        assert_eq!(
            extract_namespace("MATCH (f:core.File) RETURN f"),
            Some("core.File".into())
        );
        assert_eq!(
            extract_namespace("MATCH (p:shared.Person)-[:WORKS_AT]->(o)"),
            Some("shared.Person".into())
        );
        assert_eq!(extract_namespace("MATCH (anonymous) RETURN anonymous"), None);
    }

    #[test]
    fn read_query_in_allowlist_succeeds() {
        let c = ctx(vec!["core."], vec![]);
        assert_eq!(
            check_query(&c, "MATCH (f:core.File) RETURN f").unwrap(),
            QueryKind::Read
        );
    }

    #[test]
    fn write_query_without_write_capability_denied() {
        let c = ctx(vec!["core."], vec![]);
        assert!(check_query(&c, "CREATE (f:core.File)").is_err());
    }

    #[test]
    fn read_query_outside_allowlist_denied() {
        let c = ctx(vec!["module.com.example."], vec![]);
        assert!(check_query(&c, "MATCH (f:core.File) RETURN f").is_err());
    }

    #[test]
    fn unknown_kind_denied() {
        let c = ctx(vec!["*"], vec!["*"]);
        // No MATCH/RETURN/CREATE/etc, no labels: classified Unknown
        // and falls through to denial.
        assert!(check_query(&c, "EXPLAIN PROFILE x").is_err());
    }

    // ----- Codex S6 critical finding tests --------------------------------

    /// Multi-namespace query: every label must pass. An attacker who
    /// embeds an allowed label first and then accesses a disallowed
    /// one would have slipped past the old first-namespace check.
    #[test]
    fn multi_namespace_query_requires_every_namespace() {
        let c = ctx(vec!["module.x."], vec![]);
        // Attack shape: lure the gate with the allowed prefix, then
        // touch a forbidden namespace.
        assert!(check_query(
            &c,
            "MATCH (a:module.x.Foo) MATCH (b:core.Secret) RETURN b"
        )
        .is_err());
    }

    #[test]
    fn multi_namespace_query_all_allowed_passes() {
        let c = ctx(vec!["module.x.", "module.y."], vec![]);
        assert_eq!(
            check_query(
                &c,
                "MATCH (a:module.x.Foo) MATCH (b:module.y.Bar) RETURN a, b"
            )
            .unwrap(),
            QueryKind::Read
        );
    }

    /// `:label` inside a string literal must not count toward the
    /// namespace check. Otherwise an attacker who can name a forbidden
    /// namespace in a string parameter could either trip a false
    /// denial (DoS) or, worse, sneak a label past by escaping quotes.
    #[test]
    fn namespace_inside_string_literal_is_ignored() {
        let c = ctx(vec!["module.x."], vec![]);
        // `:core.Secret` only appears inside a string; the query
        // proper only references `module.x.Foo`.
        let q = r#"MATCH (a:module.x.Foo) WHERE a.note = "see :core.Secret" RETURN a"#;
        assert_eq!(check_query(&c, q).unwrap(), QueryKind::Read);
    }

    #[test]
    fn namespace_inside_single_quoted_string_is_ignored() {
        let c = ctx(vec!["module.x."], vec![]);
        let q = "MATCH (a:module.x.Foo) WHERE a.note = ':core.Secret' RETURN a";
        assert_eq!(check_query(&c, q).unwrap(), QueryKind::Read);
    }

    #[test]
    fn namespace_inside_line_comment_is_ignored() {
        let c = ctx(vec!["module.x."], vec![]);
        let q = "// :core.Secret\nMATCH (a:module.x.Foo) RETURN a";
        assert_eq!(check_query(&c, q).unwrap(), QueryKind::Read);
    }

    #[test]
    fn namespace_inside_block_comment_is_ignored() {
        let c = ctx(vec!["module.x."], vec![]);
        let q = "MATCH /* :core.Secret here */ (a:module.x.Foo) RETURN a";
        assert_eq!(check_query(&c, q).unwrap(), QueryKind::Read);
    }

    #[test]
    fn extract_all_namespaces_finds_every_label() {
        let ns = extract_all_namespaces(
            "MATCH (a:foo.A)-[:REL]->(b:bar.B) MATCH (c:baz.C) RETURN a, b, c",
        );
        assert_eq!(ns, vec!["foo.A", "REL", "bar.B", "baz.C"]);
    }

    #[test]
    fn extract_all_namespaces_handles_double_colon_type_cast() {
        // Cypher type casts use `::`. We collapse to one logical
        // marker so the cast target is captured (e.g. `value::INT`
        // produces "INT" not nothing).
        let ns = extract_all_namespaces("RETURN x::INT");
        assert_eq!(ns, vec!["INT"]);
    }

    #[test]
    fn extract_all_namespaces_skips_escaped_quotes() {
        // A quoted string with a `\"` escape must not terminate the
        // string scan early, otherwise a label after the escape
        // would leak out.
        let ns = extract_all_namespaces(r#"MATCH (a:foo.A) WHERE a.x = "he said \":bar.B\"" RETURN a"#);
        assert_eq!(ns, vec!["foo.A"]);
    }
}
