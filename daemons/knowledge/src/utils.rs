/// Escape a string for safe interpolation into a Cypher single-quoted literal.
///
/// Cypher uses `'...'` for string literals with `\` as the escape character.
/// This function escapes backslashes and single quotes so that user-supplied
/// values (file paths, app IDs, window titles) cannot break out of the string.
pub fn escape_cypher(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
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
