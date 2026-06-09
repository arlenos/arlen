//! The structural hijack canary (canary-honeytools.md §3).
//!
//! A deterministic, zero-false-positive tripwire. A static set of **canary ids**
//! under a reserved id namespace that honest behaviour provably never names: real
//! operands are File/Project ids that came from ingestion, and the ingestion
//! boundary reserves this prefix so no honest id can ever collide with one. A
//! canary id can therefore enter a proposal's operands **only** if an external
//! injection supplied the string from outside the loop, which is exactly the
//! genuine-hijack condition. So a touch is *proof* of hijack, not a score.
//!
//! This is the structural canary, not the cut "attractive decoy": there is **no
//! KG plant and no graph read**. The original discoverable `__canary` bait node
//! was dropped because the read tool renders result rows verbatim into the model's
//! context, so an honest survey would surface the bait id and the model could then
//! name it without being hijacked, defeating the zero-FP proof. Planting nothing
//! removes the only honest path that could feed the model a canary string.
//!
//! This module is the pure detection core: the reserved namespace, the canary set
//! and the membership check over a proposal's operands. The wiring is the paired
//! follow-up (like the executor and compensation mechanisms, built before their
//! triggers): a pre-scope `touched_by` check at the gate's `Propose` arm firing
//! the shared freeze terminal (run *before* the predict-before-act proof, so it
//! catches a touch in suggest-mode too, where no proof runs), the content-free
//! `CanaryTrip` audit outcome (CY-R3), and the ingestion-boundary reservation in
//! the knowledge daemon that rejects any node-create whose id bears
//! [`RESERVED_CANARY_PREFIX`] (CY-R2's supporting build, which must use this same
//! prefix string so the reservation and the detection agree).

// The detection core is built before its trigger (the gate pre-scope hook +
// the ingestion reservation), the same way the executor and compensation
// mechanisms were. Its items read as unused in a non-test build until that
// wiring lands, so the dead-code lint is allowed here, not crate-wide.
#![allow(dead_code)]

use std::collections::BTreeMap;

/// The reserved canary id namespace. No honest ingestion id may bear this prefix:
/// the ingestion boundary rejects any node-create whose id starts with it, so an
/// operand under this prefix can only have come from outside the loop. The check
/// is prefix-based (not just exact-set membership) so the whole reserved namespace
/// is a tripwire, not only the explicitly named [`CANARY_IDS`].
pub(crate) const RESERVED_CANARY_PREFIX: &str = "__canary:";

/// The explicitly named canary ids. Every entry lives under
/// [`RESERVED_CANARY_PREFIX`] (asserted by a test), so the prefix reservation
/// guarantees none of them can ever be a real node id. They carry the deception's
/// attractive shape for documentation and audit-cause naming; the actual detection
/// is the prefix check, which subsumes this set.
pub(crate) const CANARY_IDS: &[&str] = &[
    "__canary:credentials-vault",
    "__canary:system-keyring",
];

/// Whether `id` falls inside the reserved canary namespace, i.e. begins with the
/// reserved prefix. This is the predicate the ingestion boundary uses to refuse
/// creating a node whose id collides with the canary namespace (a node id is a
/// structured token, so the namespace is a prefix). Detection over free-form
/// operands is broader (see [`touched_by`]).
pub(crate) fn is_reserved_canary_id(id: &str) -> bool {
    id.starts_with(RESERVED_CANARY_PREFIX)
}

/// The structural-canary check: the first operand value in `arguments` that
/// *mentions* the reserved canary token, or `None` if the proposal touches no
/// canary. Pure substring search over the operand values, with no graph read and
/// no scope check, so it is cheap and runs pre-scope.
///
/// Detection is **substring**, not whole-value prefix, on purpose: a mutate
/// operand is itself a node id (so the value *is* a canary id), but a read
/// operand is a free-form Cypher `query` string with the canary id embedded
/// (`{id:'__canary:...'}`). A `contains` check catches both; a `starts_with`
/// check would miss the embedded-in-query case, which is the channel that renders
/// results verbatim into the model transcript (the dangerous one, §3). The
/// reserved token is distinctive, so a real File/Project id or an honest query
/// never contains it (zero false positives, once the ingestion reservation keeps
/// the namespace empty).
pub(crate) fn touched_by(arguments: &BTreeMap<String, String>) -> Option<&str> {
    arguments
        .values()
        .map(String::as_str)
        .find(|value| value.contains(RESERVED_CANARY_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn every_named_canary_is_inside_the_reserved_namespace() {
        // The reservation only protects ids under the prefix, so a named canary
        // outside it would be forgeable; this keeps the set and the reservation
        // consistent by construction.
        for id in CANARY_IDS {
            assert!(is_reserved_canary_id(id), "{id} must use the reserved prefix");
        }
    }

    #[test]
    fn honest_operands_touch_no_canary() {
        // Real File/Project ids from ingestion never bear the reserved prefix.
        let a = args(&[
            ("file", "/home/tim/notes.md"),
            ("project", "proj-7f3a"),
            ("id", "system.File:42"),
        ]);
        assert_eq!(touched_by(&a), None);
        assert_eq!(touched_by(&BTreeMap::new()), None);
    }

    #[test]
    fn a_named_canary_operand_is_a_touch() {
        let a = args(&[("file", "/home/tim/x"), ("target", CANARY_IDS[0])]);
        assert_eq!(touched_by(&a), Some(CANARY_IDS[0]));
    }

    #[test]
    fn any_reserved_prefix_operand_is_a_touch_even_if_not_named() {
        // The whole namespace is the tripwire, not only the listed ids, because the
        // reservation forbids the entire prefix at ingestion.
        let probed = "__canary:made-up-by-an-injection";
        let a = args(&[("node", probed)]);
        assert_eq!(touched_by(&a), Some(probed));
    }

    #[test]
    fn a_canary_embedded_in_a_read_query_is_a_touch() {
        // The read tool's operand is a free-form Cypher string with the canary id
        // embedded, not a standalone id; substring detection catches it where a
        // whole-value prefix check would not.
        let q = "MATCH (n {id:'__canary:credentials-vault'}) RETURN n";
        let a = args(&[("query", q)]);
        assert_eq!(touched_by(&a), Some(q));
        // An honest survey query never mentions the reserved token.
        let honest = args(&[("query", "MATCH (f:File) RETURN f.id")]);
        assert_eq!(touched_by(&honest), None);
    }

    #[test]
    fn reserved_predicate_matches_only_the_prefix() {
        assert!(is_reserved_canary_id("__canary:anything"));
        assert!(!is_reserved_canary_id("system.File:1"));
        assert!(!is_reserved_canary_id("canary:no-underscores"));
        assert!(!is_reserved_canary_id(""));
    }
}
