//! The delegated namespace grant: the authorization primitive a foreign-app
//! bridge writes the Knowledge Graph under (foreign-app-bridges.md §2). It is the
//! pure core of the namespace-prefix capability check (DECIDED 22 Jul to be a flat
//! prefix check, NOT a macaroon).
//!
//! The ordinary write path binds a caller to its OWN namespace: `check_namespace`
//! (create.rs) admits an `entity_type` only when it starts with `{app_id}.`. A
//! bridge breaks that 1:1 binding on purpose - the Obsidian bridge process is
//! attested as its own app id (e.g. `bridge-ingest`) but must write
//! `md.obsidian.Note` nodes. A grant resolves it: the bridge holds a grant for a
//! DELEGATED namespace (`md.obsidian`), and a write is admitted when its type lies
//! under that granted namespace, not under the caller's app id.
//!
//! This module is the pure authorization core - the grant vocabulary and its
//! checks - with three invariants (foreign-app-bridges.md §2):
//!
//! 1. **`system.*` / `shared.*` are structurally ungrantable.** [`NamespaceGrant::new`]
//!    refuses to mint a grant for a reserved namespace, so no caveat can ever reach
//!    a system or shared fact - the anti-poisoning guarantee. A third-party bridge
//!    can never forge a system-origin node.
//! 2. **Fixed scope, never widen.** A grant covers exactly one namespace and its
//!    sub-tree; there is no sub-delegation. Bridge write-scope was DECIDED (22 Jul,
//!    foreign-app-bridges.md) to be the flat namespace-prefix check, NOT a macaroon,
//!    so the chained-attenuation primitive is gone rather than dormant.
//! 3. **Fail closed.** An empty, reserved, or malformed prefix yields no grant; a
//!    type that is not strictly under the granted namespace is not permitted.
//!
//! It does NO I/O and is independent of how the grant is DELIVERED (a scoped token
//! carrying the raw prefix strings). The write-path consumer has LANDED:
//! [`permits_any`] gates both the `UpsertEntity` and the `LinkEntities` paths
//! (`write/entity.rs`), admitting a write whose type is under the caller's delegated
//! grant in addition to its own app-id namespace.

/// The reserved namespaces no grant may ever cover: a bridge can never be
/// delegated authority over system- or shared-owned facts (foreign-app-bridges.md
/// §2 - `system.*`/`shared.*` are structurally unwritable by a third party).
const RESERVED_NAMESPACES: &[&str] = &["system", "shared"];

/// A delegated namespace grant: the namespace prefix a holder may write entity
/// types under, even though it is not the holder's own app id. Construct only via
/// [`NamespaceGrant::new`] (fail-closed), so an invalid or reserved grant is
/// unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceGrant {
    /// The granted namespace, e.g. `md.obsidian`. Never reserved, never empty,
    /// always a valid reverse-DNS-shaped identifier (the `new` floor). Do NOT
    /// construct a `NamespaceGrant` by struct literal anywhere - the field is
    /// module-private precisely so `new` is the only way to mint one, keeping the
    /// reserved-namespace + validity floor unbypassable. A literal here would skip
    /// that floor (e.g. mint a `system`-covering grant).
    prefix: String,
}

impl NamespaceGrant {
    /// Mint a grant for `prefix`, fail-closed. Returns `None` for an empty prefix,
    /// a reserved namespace (`system`/`shared`, exact or as a leading segment), or
    /// a prefix that is not a valid namespace identifier (lowercase reverse-DNS:
    /// `[a-z0-9-]` segments joined by `.`, no leading/trailing/empty segment) - so
    /// a malformed or reserved grant can never be held.
    pub fn new(prefix: &str) -> Option<NamespaceGrant> {
        if !is_valid_namespace(prefix) || is_reserved(prefix) {
            return None;
        }
        Some(NamespaceGrant {
            prefix: prefix.to_string(),
        })
    }

    /// The granted namespace prefix (`md.obsidian`). No live caller yet (the write
    /// path checks via [`permits`](Self::permits)); kept as the natural accessor a
    /// grant-introspection or chained-delegation consumer will use.
    #[allow(dead_code)]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Whether this grant permits writing `entity_type`. An entity type is a
    /// dotted `{namespace}.{Type}`, so the type must lie STRICTLY under the granted
    /// namespace (`md.obsidian` permits `md.obsidian.Note`, never `md.obsidian`
    /// itself nor a sibling like `md.obsidianvault.Note`). The dotted boundary is
    /// load-bearing: a bare `starts_with` would let `md.obsidian` grant
    /// `md.obsidianX.Note`.
    ///
    /// This is a NAMESPACE-boundary gate, NOT a full type-shape check: it answers
    /// only "is this type strictly under my granted namespace?". A permitted type
    /// is always strictly under the grant (never a sibling, never `system.*`/
    /// `shared.*`), but its full validity - a registered, well-formed `{ns}.{Type}`
    /// - is the schema registry's separate check the write-path wiring keeps.
    pub fn permits(&self, entity_type: &str) -> bool {
        match entity_type.strip_prefix(&self.prefix) {
            // A `.` boundary then a type segment whose first byte is an identifier
            // char. Requiring an alphanumeric start rejects degenerate tails at the
            // boundary (`md.obsidian..`, `md.obsidian. `, a control char) while
            // accepting both a PascalCase type (`.Note`) and a lowercase
            // sub-namespace segment (`.vault`).
            Some(rest) => {
                let mut bytes = rest.bytes();
                bytes.next() == Some(b'.') && bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
            }
            None => false,
        }
    }

}

/// Whether ANY of the caller's declared delegated namespaces grants writing
/// `entity_type`. `delegated` is the raw prefix strings from the caller's profile
/// (e.g. `["md.obsidian"]`), carried verbatim on the token so the grant type stays
/// un-serializable (sealed). Each declared string is validated through
/// [`NamespaceGrant::new`] HERE - the authoritative, fail-closed gate - so a
/// reserved (`system`/`shared`) or malformed declared namespace yields no grant and
/// permits nothing. This is the write path's delegation hook: a write whose type is
/// not under the caller's own app-id namespace is admitted only if some delegated
/// grant permits it.
///
/// TRACKED follow-up (adversarial review, MEDIUM): a grant checks only that the type
/// is strictly under the delegated namespace, NOT that the namespace is UNOWNED. A
/// user profile that delegated an installed app's namespace (e.g. a bridge granted
/// `com.other`) would let the bridge write into that app's own entity table/ids (the
/// table + node id key on the qualified type, not `_owner`), MERGE-clobbering its
/// instances. Bounded today - it cannot reach `system.*`/`shared.*`, it is
/// app-tier-within-the-user's-own-KG, gated behind the user-owned profile (the
/// same-uid F3 residual), and the custom-type READ path is unwired so there is no
/// read-poisoning consumer yet. The fix (reject a delegation matching an installed
/// app's namespace, or owner-scope the entity table/id) lands WITH the custom-type
/// read path - the intended delegation is a bridge's OWN external-tool namespace
/// (`md.obsidian`), not another installed app's.
pub fn permits_any(delegated: &[String], entity_type: &str) -> bool {
    delegated
        .iter()
        .any(|ns| NamespaceGrant::new(ns).is_some_and(|g| g.permits(entity_type)))
}

/// Whether `ns` is or starts with a reserved namespace segment (`system`/`shared`).
/// Segment-aware: `system` and `system.x` are reserved, `systematic` is not.
fn is_reserved(ns: &str) -> bool {
    RESERVED_NAMESPACES.iter().any(|r| {
        ns == *r || ns.strip_prefix(r).is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Whether `ns` is a valid namespace identifier: one or more `.`-joined segments,
/// each a non-empty run of `[a-z0-9-]` (lowercase reverse-DNS). No leading/trailing
/// dot, no empty segment, no uppercase or other punctuation - so a grant prefix is
/// a clean identifier (it is later compared as a string prefix; this keeps it from
/// being whitespace, a path, or anything that could surprise a downstream check).
fn is_valid_namespace(ns: &str) -> bool {
    if ns.is_empty() {
        return false;
    }
    ns.split('.').all(|seg| {
        !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_permits_only_types_strictly_under_it() {
        let g = NamespaceGrant::new("md.obsidian").expect("valid grant");
        assert!(g.permits("md.obsidian.Note"));
        assert!(g.permits("md.obsidian.Tag"));
        // The namespace itself is not a type.
        assert!(!g.permits("md.obsidian"));
        // A sibling that merely shares the string prefix is NOT under it (the
        // dotted boundary - this is the load-bearing case).
        assert!(!g.permits("md.obsidianvault.Note"));
        // An unrelated namespace.
        assert!(!g.permits("com.evil.Note"));
        // Trailing dot with no type segment.
        assert!(!g.permits("md.obsidian."));
        // Degenerate tails at the boundary are rejected (not authorization
        // escapes - they are still under the namespace - but a permitted type must
        // at least start a real segment; full shape is the schema's job).
        assert!(!g.permits("md.obsidian.."));
        assert!(!g.permits("md.obsidian. "));
        assert!(!g.permits("md.obsidian.\u{0}"));
    }

    #[test]
    fn no_grant_ever_permits_a_reserved_type() {
        // The load-bearing anti-poisoning property, asserted directly (not only
        // implied by the ungrantable-prefix test): a normal grant - and a deep one
        // - can never reach a system.*/shared.* type.
        let g = NamespaceGrant::new("md.obsidian").unwrap();
        assert!(!g.permits("system.File"));
        assert!(!g.permits("shared.Person"));
        let deep = NamespaceGrant::new("a.b.c.d").unwrap();
        assert!(deep.permits("a.b.c.d.Note"));
        assert!(!deep.permits("system.File"));
        assert!(!deep.permits("shared.Person"));
        // A grant whose OWN namespace merely contains the word is fine and still
        // cannot reach the top-level reserved namespace.
        let mid = NamespaceGrant::new("md.system").unwrap();
        assert!(mid.permits("md.system.Thing"));
        assert!(!mid.permits("system.File"));
    }

    #[test]
    fn reserved_namespaces_are_ungrantable() {
        assert!(NamespaceGrant::new("system").is_none());
        assert!(NamespaceGrant::new("system.File").is_none());
        assert!(NamespaceGrant::new("shared").is_none());
        assert!(NamespaceGrant::new("shared.Person").is_none());
        // A namespace that merely starts with the same letters is fine.
        assert!(NamespaceGrant::new("systematic.thing").is_some());
        assert!(NamespaceGrant::new("sharedrive.app").is_some());
    }

    #[test]
    fn malformed_prefixes_yield_no_grant() {
        assert!(NamespaceGrant::new("").is_none());
        assert!(NamespaceGrant::new("md..obsidian").is_none()); // empty segment
        assert!(NamespaceGrant::new(".md").is_none()); // leading dot
        assert!(NamespaceGrant::new("md.").is_none()); // trailing dot
        assert!(NamespaceGrant::new("md.Obsidian").is_none()); // uppercase
        assert!(NamespaceGrant::new("md obsidian").is_none()); // whitespace
        assert!(NamespaceGrant::new("md.obsidian/note").is_none()); // path char
    }

    #[test]
    fn permits_any_admits_a_delegated_namespace_and_refuses_reserved_or_foreign() {
        let delegated = vec!["md.obsidian".to_string()];
        // A type under the delegated namespace is admitted.
        assert!(permits_any(&delegated, "md.obsidian.Note"));
        // Foreign / reserved / bare-namespace types are refused.
        assert!(!permits_any(&delegated, "com.evil.Note"));
        assert!(!permits_any(&delegated, "system.File"));
        assert!(!permits_any(&delegated, "shared.Person"));
        assert!(!permits_any(&delegated, "md.obsidian"));
        // A reserved or malformed DECLARED namespace yields no grant -> permits
        // nothing (the declaration cannot smuggle authority past `new`).
        assert!(!permits_any(&["system".to_string()], "system.File"));
        assert!(!permits_any(&["md..bad".to_string()], "md..bad.X"));
        // Empty delegation permits nothing.
        assert!(!permits_any(&[], "md.obsidian.Note"));
    }

}
