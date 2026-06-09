//! Profile-first capability revoke (living-capability-graph.md §6).
//!
//! Revoke is the mutation half of the Living Capability Graph, and its security
//! property is an asymmetry: the grant projection is **authoritative for removal
//! and derivative for addition**. Removing a reach can never grant authority, so
//! a revoke is safe in direction; adding one must never be a free graph write.
//! The request is a **closed enum** ([`RevokedReach`]) with no variant that adds
//! a reach, so widening is structurally unexpressible; and the handler proves the
//! result strictly shrank with the [`is_strict_narrowing`] gate before writing,
//! so even a future narrowing bug cannot widen through this path.
//!
//! This module is the mechanism-independent core: the request vocabulary and the
//! safety gate over re-derived token scopes. The profile mutation itself is
//! **format-preserving `toml_edit`** on `~/.config/permissions/{app}.toml`, not a
//! re-serialize of the SDK `PermissionProfile`: the SDK profile's graph
//! permissions model only `read`/`write` (not the `relations` / `instance_scope`
//! the daemon's profile and the on-disk format carry), so a serialize round-trip
//! would silently drop those fields. An in-place edit cannot lose a field it does
//! not model. The handler, the file write, and the tier/required checks are
//! later increments built on this.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::token::{EntityScope, InstanceScope, RelationScope};

/// A single reach to remove. A closed enum: there is no variant that *adds* a
/// reach, so a revoke request cannot widen authority by construction (§6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevokedReach {
    /// Remove an entry of `[graph].read` (an entity-type read pattern).
    Read {
        /// The read pattern to remove.
        entity_pattern: String,
    },
    /// Remove an entry of `[graph].write`.
    Write {
        /// The write pattern to remove.
        entity_pattern: String,
    },
    /// Remove a `[graph].relations` entry (a permitted relation creation).
    Relation {
        /// The relation's source entity type.
        from: String,
        /// The relation's target entity type.
        to: String,
        /// The relation type.
        relation_type: String,
    },
    /// Demote `instance_scope` from `All` to `Own` (the app loses cross-app reach).
    InstanceAll,
}

/// Who initiated the revoke. The agent may only *propose* (suggest-mode records a
/// proposal into the pull-review timeline); the user confirming replays it as
/// [`RevokeInitiator::User`]. There is no agent path that auto-applies (§6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevokeInitiator {
    /// A user-confirmed revoke.
    User,
    /// An agent suggestion, carrying the proposal id it replays.
    Agent {
        /// The suggestion id this revoke replays.
        suggestion_id: String,
    },
}

/// A revoke request. The closed [`RevokedReach`] makes widening unexpressible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeReach {
    /// The app whose reach is narrowed.
    pub target_app_id: String,
    /// The reach to remove.
    pub reach: RevokedReach,
    /// Who initiated it.
    pub initiator: RevokeInitiator,
}

/// A set-shaped summary of a profile's re-derived runtime token scopes, the form
/// the subset gate compares. The coarse `RevokedReach` variants remove whole
/// entries (a read/write type, a relation, the `All` instance reach), so a
/// type-and-relation-keyed set is the right granularity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSummary {
    /// The readable entity types.
    pub read: BTreeSet<String>,
    /// The writable entity types.
    pub write: BTreeSet<String>,
    /// The permitted relations, keyed `(from, to, relation_type)`.
    pub relations: BTreeSet<(String, String, String)>,
    /// Whether the instance scope is `All` (the wider of the two).
    pub instance_all: bool,
}

impl ScopeSummary {
    /// Summarise a token's four scope collections into the comparable set form.
    pub fn from_scopes(
        read: &[EntityScope],
        write: &[EntityScope],
        relations: &[RelationScope],
        instance: InstanceScope,
    ) -> ScopeSummary {
        ScopeSummary {
            read: read.iter().map(|s| s.entity_type.clone()).collect(),
            write: write.iter().map(|s| s.entity_type.clone()).collect(),
            relations: relations
                .iter()
                .map(|r| (r.from.clone(), r.to.clone(), r.relation_type.clone()))
                .collect(),
            instance_all: matches!(instance, InstanceScope::All),
        }
    }
}

/// The strict-subset safety gate (§6): the `new` scope set must be a subset of
/// `old` in every dimension AND strictly smaller in at least one. It returns
/// false (refuse, `NotNarrowing`) if authority did not strictly shrink, so a
/// no-op edit, a widening, or a bug that left authority equal or larger writes
/// nothing. The closed request enum already makes widening unexpressible; this
/// proves narrowing on the actual re-derived scopes regardless.
pub fn is_strict_narrowing(old: &ScopeSummary, new: &ScopeSummary) -> bool {
    let read_subset = new.read.is_subset(&old.read);
    let write_subset = new.write.is_subset(&old.write);
    let relations_subset = new.relations.is_subset(&old.relations);
    // Instance: `All` is wider than `Own`. The new scope may not gain `All`.
    let instance_subset = !new.instance_all || old.instance_all;
    let every_dimension_subset = read_subset && write_subset && relations_subset && instance_subset;

    let strictly_smaller = new.read.len() < old.read.len()
        || new.write.len() < old.write.len()
        || new.relations.len() < old.relations.len()
        || (old.instance_all && !new.instance_all);

    every_dimension_subset && strictly_smaller
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(read: &[&str], write: &[&str], instance_all: bool) -> ScopeSummary {
        ScopeSummary {
            read: read.iter().map(|s| s.to_string()).collect(),
            write: write.iter().map(|s| s.to_string()).collect(),
            relations: BTreeSet::new(),
            instance_all,
        }
    }

    #[test]
    fn removing_a_read_type_is_strict_narrowing() {
        let old = summary(&["system.File", "system.Project"], &[], false);
        let new = summary(&["system.File"], &[], false);
        assert!(is_strict_narrowing(&old, &new));
    }

    #[test]
    fn demoting_instance_all_to_own_is_strict_narrowing() {
        let old = summary(&["system.File"], &[], true);
        let new = summary(&["system.File"], &[], false);
        assert!(is_strict_narrowing(&old, &new));
    }

    #[test]
    fn an_unchanged_scope_is_not_narrowing() {
        let s = summary(&["system.File"], &["system.File"], true);
        assert!(!is_strict_narrowing(&s, &s.clone()), "a no-op must be refused");
    }

    #[test]
    fn adding_a_read_type_is_not_narrowing() {
        let old = summary(&["system.File"], &[], false);
        let new = summary(&["system.File", "system.Project"], &[], false);
        assert!(!is_strict_narrowing(&old, &new), "a widening must be refused");
    }

    #[test]
    fn gaining_instance_all_is_not_narrowing() {
        let old = summary(&["system.File"], &[], false);
        let new = summary(&["system.File"], &[], true);
        assert!(!is_strict_narrowing(&old, &new), "gaining All is a widening");
    }

    #[test]
    fn narrowing_one_dimension_while_widening_another_is_refused() {
        // Drops a read type but adds a write type: not a subset in every
        // dimension, so refused even though one dimension shrank.
        let old = summary(&["system.File", "system.Project"], &[], false);
        let new = summary(&["system.File"], &["system.Event"], false);
        assert!(!is_strict_narrowing(&old, &new));
    }

    #[test]
    fn removing_a_relation_is_strict_narrowing() {
        let mut old = summary(&[], &[], false);
        old.relations.insert((
            "system.File".into(),
            "system.Project".into(),
            "FILE_PART_OF".into(),
        ));
        let new = summary(&[], &[], false);
        assert!(is_strict_narrowing(&old, &new));
    }

    #[test]
    fn from_scopes_summarises_each_collection() {
        let read = vec![EntityScope {
            entity_type: "system.File".into(),
            fields: None,
            exclude_fields: vec![],
        }];
        let rels = vec![RelationScope {
            from: "system.File".into(),
            to: "system.Project".into(),
            relation_type: "FILE_PART_OF".into(),
        }];
        let s = ScopeSummary::from_scopes(&read, &[], &rels, InstanceScope::All);
        assert!(s.read.contains("system.File"));
        assert!(s.write.is_empty());
        assert!(s.relations.contains(&(
            "system.File".to_string(),
            "system.Project".to_string(),
            "FILE_PART_OF".to_string()
        )));
        assert!(s.instance_all);
    }
}
