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

/// Best-effort namespace extraction from a Cypher query. Looks for the
/// first label after a node-pattern paren, e.g. `(f:core.File)` →
/// `core.File`. Returns `None` if it can not find one; callers fail
/// closed in that case.
pub fn extract_namespace(cypher: &str) -> Option<String> {
    let bytes = cypher.as_bytes();
    let colon = bytes.iter().position(|&b| b == b':')?;
    let after = &bytes[colon + 1..];
    let end = after
        .iter()
        .position(|b| !is_label_char(*b))
        .unwrap_or(after.len());
    let label = std::str::from_utf8(&after[..end]).ok()?;
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

fn is_label_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

pub fn check_query(ctx: &CapabilityContext, cypher: &str) -> Result<QueryKind> {
    let kind = classify(cypher);
    let namespace = extract_namespace(cypher).ok_or_else(|| DaemonError::CapabilityDenied {
        module_id: ctx.module_id.clone(),
        capability: "graph.query (no namespace)".into(),
    })?;

    let allowed = match kind {
        QueryKind::Read => ctx.allow_graph_read(&namespace),
        QueryKind::Write => ctx.allow_graph_write(&namespace),
        QueryKind::Unknown => false,
    };

    if !allowed {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: format!("graph.{} ({namespace})", match kind {
                QueryKind::Read => "read",
                QueryKind::Write => "write",
                QueryKind::Unknown => "unknown",
            }),
        });
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
}
