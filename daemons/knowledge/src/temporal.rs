//! Bi-temporal read predicates: the one home for the as-of liveness query
//! (bitemporal-knowledge-graph.md §4.4, §7.5).
//!
//! Every default read of a temporalised edge is gated by one predicate,
//! [`valid_as_of`], parameterised by `T_asof` (default the current instant). It
//! selects an edge that is true in the world at `T_asof` (valid axis), that
//! Arlen knew by `T_asof` (transaction axis), and whose BOTH endpoint nodes
//! existed as of `T_asof` (node transaction liveness) — the last clause closes
//! the node-vs-edge gap where an edge to an archived Project still read as live.
//! Point-in-time and delta are the same predicate with different parameters, not
//! new code paths: point-in-time sets `T_asof = T`; delta ([`delta`]) is a range
//! filter on the transaction axis. History is read, never replayed, because
//! invalidation closes an edge but never deletes it.
//!
//! These emit WHERE-clause fragments only (no `MATCH`/`CREATE`/`SET`), so they
//! never trip the read socket's write-query guard. The variable names are caller
//! supplied and the instants are query parameters, so no value is interpolated.

/// Emit the `VALID_AS_OF` liveness predicate for a relationship pattern bound as
/// `rel` between endpoint nodes bound `from` and `to`, evaluated at the query
/// parameter `$<t_param>`.
///
/// With `with_hindsight` false (the default, honest as-of) every clause is
/// present: a fact learned *after* `T_asof` about a time before it must not
/// appear, because the agent did not know it then. With `with_hindsight` true
/// ("what was actually true in the world, with all hindsight") the
/// transaction-axis clauses are dropped — the edge's `created_at`/`expired_at`
/// and both endpoints' `expired_at` — keeping only the valid axis.
pub fn valid_as_of(rel: &str, from: &str, to: &str, t_param: &str, with_hindsight: bool) -> String {
    // Valid axis: became true at or before T_asof and had not become false by then.
    let valid = format!(
        "{rel}.valid_at <= ${t} AND ({rel}.invalid_at IS NULL OR {rel}.invalid_at > ${t})",
        rel = rel,
        t = t_param,
    );
    if with_hindsight {
        return valid;
    }
    // Transaction axis on the edge + both endpoint nodes' transaction liveness.
    format!(
        "{valid} \
         AND {rel}.created_at <= ${t} AND ({rel}.expired_at IS NULL OR {rel}.expired_at > ${t}) \
         AND ({from}.expired_at IS NULL OR {from}.expired_at > ${t}) \
         AND ({to}.expired_at IS NULL OR {to}.expired_at > ${t})",
        valid = valid,
        rel = rel,
        from = from,
        to = to,
        t = t_param,
    )
}

/// Emit the transaction-axis delta filter (§7.5): edges whose learning state
/// changed within the half-open window `($<lo_param>, $<hi_param>]`. With
/// `newly_learned` true this selects `created_at` (facts Arlen learned in the
/// window); false selects `expired_at` (facts retracted in the window). A NULL
/// `expired_at` (a still-believed fact) is correctly excluded, since a NULL
/// comparison is not true.
pub fn delta(rel: &str, lo_param: &str, hi_param: &str, newly_learned: bool) -> String {
    let col = if newly_learned { "created_at" } else { "expired_at" };
    format!(
        "{rel}.{col} > ${lo} AND {rel}.{col} <= ${hi}",
        rel = rel,
        col = col,
        lo = lo_param,
        hi = hi_param,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The write keywords the read socket's guard rejects; a read predicate must
    /// contain none of them (§12 R3: confirm the shapes do not trip the guard).
    const WRITE_KEYWORDS: [&str; 15] = [
        "CREATE", "MERGE", "DELETE", "SET", "REMOVE", "DROP", "DETACH", "ALTER",
        "ATTACH", "USE", "COPY", "LOAD", "INSTALL", "EXPORT", "IMPORT",
    ];

    fn has_no_write_keyword(fragment: &str) -> bool {
        let upper = fragment.to_ascii_uppercase();
        // Word-ish check: surround with spaces so a substring like "USE" inside
        // a variable cannot false-match; the fragments use no such identifiers.
        WRITE_KEYWORDS
            .iter()
            .all(|kw| !format!(" {upper} ").contains(&format!(" {kw} ")))
    }

    #[test]
    fn default_predicate_has_both_axes_and_both_endpoint_clauses() {
        let p = valid_as_of("r", "a", "b", "t_asof", false);
        assert!(p.contains("r.valid_at <= $t_asof"));
        assert!(p.contains("r.invalid_at IS NULL OR r.invalid_at > $t_asof"));
        assert!(p.contains("r.created_at <= $t_asof"));
        assert!(p.contains("r.expired_at IS NULL OR r.expired_at > $t_asof"));
        assert!(p.contains("a.expired_at IS NULL OR a.expired_at > $t_asof"));
        assert!(p.contains("b.expired_at IS NULL OR b.expired_at > $t_asof"));
    }

    #[test]
    fn hindsight_drops_every_transaction_clause_keeping_the_valid_axis() {
        let p = valid_as_of("r", "a", "b", "t", true);
        assert!(p.contains("r.valid_at <= $t"));
        assert!(p.contains("r.invalid_at IS NULL"));
        assert!(!p.contains("created_at"), "edge transaction clause dropped");
        assert!(!p.contains("expired_at"), "edge + node transaction clauses dropped");
        assert!(!p.contains("a.") && !p.contains("b."), "node clauses dropped");
    }

    #[test]
    fn delta_selects_the_right_transaction_column() {
        assert!(delta("r", "lo", "hi", true).contains("r.created_at > $lo AND r.created_at <= $hi"));
        assert!(delta("r", "lo", "hi", false).contains("r.expired_at > $lo AND r.expired_at <= $hi"));
    }

    #[test]
    fn the_predicates_trip_no_write_guard() {
        assert!(has_no_write_keyword(&valid_as_of("r", "a", "b", "t", false)));
        assert!(has_no_write_keyword(&valid_as_of("r", "a", "b", "t", true)));
        assert!(has_no_write_keyword(&delta("r", "lo", "hi", true)));
    }
}
