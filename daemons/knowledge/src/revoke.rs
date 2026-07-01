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
//! not model.
//!
//! **Honest limits (both fail toward "didn't shrink enough", never toward a
//! widening, so neither is an authority leak):**
//! - **Required-reach refusal (§6.3) is NOT yet enforced here.** The design says
//!   an essential reach is refused so a one-click revoke cannot brick an app, but
//!   the on-disk `GraphPermissions` carries no `required` marker today, so nothing
//!   is markable required and the refusal is inert-by-absence. When the schema
//!   gains a `required` field, wire the refusal in `handle_revoke` before the
//!   gate; until then the gate will happily approve removing any reach.
//! - **Revoking a pattern still covered by a live wildcard is cosmetic.** The gate
//!   compares the raw pattern entries, so removing `system.File` while `system.*`
//!   remains is a strict-subset shrink (reported `Revoked`) even though the
//!   effective coverage is unchanged. Surfacing "still covered by `system.*`"
//!   needs the wildcard-expansion (`pattern_matches`) the gate deliberately does
//!   not do; a follow-up, recorded here so the `Revoked` outcome is not mistaken
//!   for "this reach is now gone" when a broader entry still grants it.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::permission::PermissionProfile;
use crate::quota::{AppTier, QuotaConfig};
use crate::token::InstanceScope;

// The revoke wire-contract types (the request enums + the outcome) live in
// `arlen-permissions`, shared with the os-sdk client so the request shape and the
// outcome tokens have one definition and cannot drift. Re-exported so the daemon's
// `crate::revoke::{RevokeReach, RevokedReach, RevokeInitiator, RevokeOutcome}`
// references resolve unchanged; the daemon-internal logic (the gate, the
// `toml_edit` narrowing, the command) stays here.
pub use arlen_permissions::revoke::{
    RestoreOutcome, RevokeInitiator, RevokeOutcome, RevokeReach, RevokedReach,
};

/// A set-shaped summary of a profile's re-derived runtime token scopes, the form
/// the subset gate compares. The coarse `RevokedReach` variants remove whole
/// entries (a read/write type, a relation, the `All` instance reach), so a
/// type-and-relation-keyed set is the right granularity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSummary {
    /// The raw `[graph].read` pattern entries (e.g. `system.File`,
    /// `system.File.path`, `com.x.*`). Kept as the verbatim patterns, NOT
    /// collapsed to entity types: a field-scoped pattern (`system.File.path`)
    /// and a broader one (`system.File`) are distinct entries, so removing one
    /// while another for the same type remains is a real narrowing the gate must
    /// see. Collapsing to entity types would hide it and false-refuse the revoke.
    pub read: BTreeSet<String>,
    /// The raw `[graph].write` pattern entries, same discipline as `read`.
    pub write: BTreeSet<String>,
    /// The raw `[graph].read_sensitive` pattern entries. Not token-bearing today
    /// (the mint reads only read/write/relations/instance), but the gate covers
    /// it so the invariant "every authority-bearing field is in the subset check"
    /// holds by construction the day this field flows into the token, rather than
    /// silently leaving a revoke able to shrink `read` while sensitive reach stays.
    pub read_sensitive: BTreeSet<String>,
    /// The permitted relations, keyed `(from, to, relation_type)`.
    pub relations: BTreeSet<(String, String, String)>,
    /// Whether the instance scope is `All` (the wider of the two).
    pub instance_all: bool,
}

impl ScopeSummary {
    /// Summarise a profile's graph reach into the comparable set form: the raw
    /// read/write pattern entries, the relation tuples, and the instance breadth.
    /// Raw patterns (not parsed entity types) so field-level narrowing is visible
    /// to the subset gate.
    pub fn from_profile(profile: &PermissionProfile) -> ScopeSummary {
        let (read, write, read_sensitive) = match &profile.graph {
            Some(g) => (
                g.read.iter().cloned().collect(),
                g.write.iter().cloned().collect(),
                g.read_sensitive.iter().cloned().collect(),
            ),
            None => (BTreeSet::new(), BTreeSet::new(), BTreeSet::new()),
        };
        ScopeSummary {
            read,
            write,
            read_sensitive,
            relations: profile
                .to_relation_scopes()
                .iter()
                .map(|r| (r.from.clone(), r.to.clone(), r.relation_type.clone()))
                .collect(),
            instance_all: matches!(profile.to_instance_scope(), InstanceScope::All),
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
    let read_sensitive_subset = new.read_sensitive.is_subset(&old.read_sensitive);
    let relations_subset = new.relations.is_subset(&old.relations);
    // Instance: `All` is wider than `Own`. The new scope may not gain `All`.
    let instance_subset = !new.instance_all || old.instance_all;
    let every_dimension_subset =
        read_subset && write_subset && read_sensitive_subset && relations_subset && instance_subset;

    let strictly_smaller = new.read.len() < old.read.len()
        || new.write.len() < old.write.len()
        || new.read_sensitive.len() < old.read_sensitive.len()
        || new.relations.len() < old.relations.len()
        || (old.instance_all && !new.instance_all);

    every_dimension_subset && strictly_smaller
}

/// Apply a revoke to a parsed profile document in place, format-preserving
/// (comments, ordering, and every field the daemon models but a re-serialize
/// might not are untouched). Returns `true` if something was actually removed or
/// demoted, `false` if the reach was already absent (a no-op the caller treats as
/// `NotNarrowing`). Operates on `toml_edit::DocumentMut` so the user's file shape
/// survives the edit.
pub fn apply_revoke(doc: &mut toml_edit::DocumentMut, reach: &RevokedReach) -> bool {
    use toml_edit::{Item, Value};

    let Some(graph) = doc.get_mut("graph").and_then(Item::as_table_like_mut) else {
        // No `[graph]` table: nothing to narrow.
        return false;
    };

    match reach {
        RevokedReach::Read { entity_pattern } => {
            remove_string_from_array(graph, "read", entity_pattern)
        }
        RevokedReach::Write { entity_pattern } => {
            remove_string_from_array(graph, "write", entity_pattern)
        }
        RevokedReach::Relation {
            from,
            to,
            relation_type,
        } => {
            // Relations may be written either as an inline array
            // (`relations = [{ from=.., to=.., type=.. }]`) or, as serde's TOML
            // serializer emits a `Vec<struct>`, as an array-of-tables
            // (`[[graph.relations]]`). Handle both, so a relation revoke is not
            // silently a no-op on a profile written in the other form.
            if let Some(arr) = graph.get_mut("relations").and_then(Item::as_array_mut) {
                let before = arr.len();
                arr.retain(|v| {
                    let matches = v
                        .as_inline_table()
                        .map(|t| {
                            inline_str(t, "from") == Some(from.as_str())
                                && inline_str(t, "to") == Some(to.as_str())
                                && inline_str(t, "type") == Some(relation_type.as_str())
                        })
                        .unwrap_or(false);
                    !matches
                });
                arr.len() != before
            } else if let Some(tables) =
                graph.get_mut("relations").and_then(Item::as_array_of_tables_mut)
            {
                let before = tables.len();
                tables.retain(|t| {
                    let table_str = |k: &str| t.get(k).and_then(|i| i.as_str());
                    let matches = table_str("from") == Some(from.as_str())
                        && table_str("to") == Some(to.as_str())
                        && table_str("type") == Some(relation_type.as_str());
                    !matches
                });
                tables.len() != before
            } else {
                false
            }
        }
        RevokedReach::InstanceAll => {
            // Demote `all` -> `own`; a no-op (false) if it was not `all`.
            let is_all = graph
                .get("instance_scope")
                .and_then(Item::as_str)
                .map(|s| s.eq_ignore_ascii_case("all"))
                .unwrap_or(false);
            if is_all {
                graph.insert("instance_scope", Item::Value(Value::from("own")));
                true
            } else {
                false
            }
        }
    }
}

/// Remove every array element equal to `value` from the `key` string array of
/// `graph` (a pattern revoked entirely, so any duplicate entries go too).
/// Returns whether an element was removed.
fn remove_string_from_array(
    graph: &mut dyn toml_edit::TableLike,
    key: &str,
    value: &str,
) -> bool {
    let Some(arr) = graph.get_mut(key).and_then(toml_edit::Item::as_array_mut) else {
        return false;
    };
    let before = arr.len();
    arr.retain(|v| v.as_str() != Some(value));
    arr.len() != before
}

/// The string value of `key` in an inline table, if present and a string.
fn inline_str<'a>(table: &'a toml_edit::InlineTable, key: &str) -> Option<&'a str> {
    table.get(key).and_then(|v| v.as_str())
}

/// The strict-superset check: the `new` scope set contains `old` in every
/// dimension AND is strictly larger in at least one. It is the mirror of
/// [`is_strict_narrowing`] and proves an edit ONLY grew authority (added a
/// reach, promoted instance to `All`) without also removing anything.
///
/// SAFETY: passing this is NOT sufficient to authorise a restore. A restore is
/// the one authority-growth path in the system, so the op layer MUST additionally
/// bound the widened result within the app's DECLARED CEILING (the Grant node's
/// `declared_ceiling`): a restore may only reinstate a capability the app already
/// declared and the user just removed, never grant one it never had. This gate is
/// the "it grew, and grew only" half; the ceiling bound is the load-bearing half
/// and lives in the reviewed daemon op, not here.
pub fn is_strict_widening(old: &ScopeSummary, new: &ScopeSummary) -> bool {
    let read_superset = old.read.is_subset(&new.read);
    let write_superset = old.write.is_subset(&new.write);
    let read_sensitive_superset = old.read_sensitive.is_subset(&new.read_sensitive);
    let relations_superset = old.relations.is_subset(&new.relations);
    // Instance: `All` is the wider reach. The new scope may not lose an `All` the
    // old had (that would be a narrowing, not a restore).
    let instance_superset = !old.instance_all || new.instance_all;
    let every_dimension_superset = read_superset
        && write_superset
        && read_sensitive_superset
        && relations_superset
        && instance_superset;

    let strictly_larger = new.read.len() > old.read.len()
        || new.write.len() > old.write.len()
        || new.read_sensitive.len() > old.read_sensitive.len()
        || new.relations.len() > old.relations.len()
        || (!old.instance_all && new.instance_all);

    every_dimension_superset && strictly_larger
}

/// Apply a restore (re-widen) to a parsed profile document in place: the reverse
/// of [`apply_revoke`], it ADDS a previously-removed reach back. Format-preserving
/// (the user's file shape, comments and every unmodelled field survive). Returns
/// `true` if something was actually added or promoted, `false` if the reach was
/// already present (a no-op). Creates the `[graph]` table if the revoke had left
/// none.
///
/// SAFETY: this is the write mechanism only. The caller (the reviewed daemon op)
/// MUST bound the reach within the app's declared ceiling before calling this and
/// prove the result strictly widened with [`is_strict_widening`]; on its own this
/// function will happily add any reach it is handed.
pub fn apply_restore(doc: &mut toml_edit::DocumentMut, reach: &RevokedReach) -> bool {
    use toml_edit::{Array, Item, Table, Value};

    // A revoke can leave a profile with no `[graph]` table (or no target array);
    // a restore must be able to re-create the entry it re-adds.
    if doc.get("graph").and_then(Item::as_table_like).is_none() {
        doc.insert("graph", Item::Table(Table::new()));
    }
    let graph = doc
        .get_mut("graph")
        .and_then(Item::as_table_like_mut)
        .expect("graph table ensured above");

    match reach {
        RevokedReach::Read { entity_pattern } => add_string_to_array(graph, "read", entity_pattern),
        RevokedReach::Write { entity_pattern } => {
            add_string_to_array(graph, "write", entity_pattern)
        }
        RevokedReach::Relation {
            from,
            to,
            relation_type,
        } => {
            // Already present in either serialization form -> no-op.
            let present_inline = graph
                .get("relations")
                .and_then(Item::as_array)
                .map(|arr| {
                    arr.iter().any(|v| {
                        v.as_inline_table()
                            .map(|t| {
                                inline_str(t, "from") == Some(from.as_str())
                                    && inline_str(t, "to") == Some(to.as_str())
                                    && inline_str(t, "type") == Some(relation_type.as_str())
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            let present_tables = graph
                .get("relations")
                .and_then(Item::as_array_of_tables)
                .map(|tables| {
                    tables.iter().any(|t| {
                        let s = |k: &str| t.get(k).and_then(|i| i.as_str());
                        s("from") == Some(from.as_str())
                            && s("to") == Some(to.as_str())
                            && s("type") == Some(relation_type.as_str())
                    })
                })
                .unwrap_or(false);
            if present_inline || present_tables {
                return false;
            }
            // Append as an inline table (valid TOML the daemon parser accepts in
            // either form); create the inline array if absent. An existing
            // array-of-tables form cannot take an inline push, so fall back to a
            // fresh inline array only when no relations key exists.
            let mut entry = toml_edit::InlineTable::new();
            entry.insert("from", Value::from(from.clone()));
            entry.insert("to", Value::from(to.clone()));
            entry.insert("type", Value::from(relation_type.clone()));
            if let Some(arr) = graph.get_mut("relations").and_then(Item::as_array_mut) {
                arr.push(Value::InlineTable(entry));
                true
            } else if graph.get("relations").is_none() {
                let mut arr = Array::new();
                arr.push(Value::InlineTable(entry));
                graph.insert("relations", Item::Value(Value::Array(arr)));
                true
            } else {
                // An array-of-tables form exists but did not contain the relation;
                // re-adding to that form is left to the op layer if ever needed.
                false
            }
        }
        RevokedReach::InstanceAll => {
            let is_all = graph
                .get("instance_scope")
                .and_then(Item::as_str)
                .map(|s| s.eq_ignore_ascii_case("all"))
                .unwrap_or(false);
            if is_all {
                false
            } else {
                graph.insert("instance_scope", Item::Value(Value::from("all")));
                true
            }
        }
    }
}

/// Add `value` to the `key` string array of `graph` if not already present,
/// creating the array if absent. Returns whether it was added (the reverse of
/// [`remove_string_from_array`]).
fn add_string_to_array(graph: &mut dyn toml_edit::TableLike, key: &str, value: &str) -> bool {
    use toml_edit::{Array, Item, Value};
    if let Some(arr) = graph.get_mut(key).and_then(Item::as_array_mut) {
        if arr.iter().any(|v| v.as_str() == Some(value)) {
            return false;
        }
        arr.push(value);
        true
    } else if graph.get(key).is_none() {
        let mut arr = Array::new();
        arr.push(value);
        graph.insert(key, Item::Value(Value::Array(arr)));
        true
    } else {
        false
    }
}

/// Summarise a loaded profile's re-derived runtime token scopes into the form
/// the subset gate compares.
fn summarize(profile: &PermissionProfile) -> ScopeSummary {
    ScopeSummary::from_profile(profile)
}

/// Whether the user tier may revoke this app's reach (§6.2): user-tier apps yes,
/// system principals no (their authority is managed by the system, not revocable
/// through this user-tier path). The system-tier store (`/var/lib`) is unbuilt,
/// so every app's profile is in `~/.config` today; this refusal keys on the
/// quota tier so a core system principal is never narrowed here.
pub fn tier_allows_revoke(app_id: &str) -> bool {
    QuotaConfig::arlen_default().tier_for_app(app_id) != AppTier::System
}

/// Apply a revoke to the profile at `path` (§6): load it, re-derive its scopes,
/// narrow the document in place, prove the result is a strict subset of the
/// original, and only then write it back atomically. Writes nothing unless the
/// gate confirms authority strictly shrank, so a no-op, a non-narrowing edit, or
/// a parse round-trip surprise leaves the file untouched.
///
/// This is the user-config-writing core; the caller (the socket command) applies
/// the tier refusal ([`tier_allows_revoke`]) and resolves `path` from the app id.
pub fn revoke_at(path: &Path, reach: &RevokedReach) -> std::io::Result<RevokeOutcome> {
    if !path.exists() {
        return Ok(RevokeOutcome::NotFound);
    }
    let old_text = std::fs::read_to_string(path)?;
    let old_profile: PermissionProfile = toml::from_str(&old_text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let old_summary = summarize(&old_profile);

    let mut doc: toml_edit::DocumentMut = old_text
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if !apply_revoke(&mut doc, reach) {
        // The reach was already absent.
        return Ok(RevokeOutcome::NoChange);
    }
    let new_text = doc.to_string();

    // The safety gate: re-derive the narrowed profile's scopes and prove they are
    // a strict subset of the original. If not, write nothing.
    let new_profile: PermissionProfile = toml::from_str(&new_text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let new_summary = summarize(&new_profile);
    if !is_strict_narrowing(&old_summary, &new_summary) {
        return Ok(RevokeOutcome::NotNarrowing);
    }

    atomic_write(path, new_text.as_bytes())?;
    Ok(RevokeOutcome::Revoked)
}

/// Whether `scope` is entirely within `ceiling` in every dimension. The declared
/// ceiling bounds how far a restore may re-widen: a restore may only reinstate a
/// capability the app already declared. Every read/write/read_sensitive pattern
/// and relation must be in the ceiling, and instance `All` requires the ceiling to
/// permit `All`.
pub fn is_within_ceiling(scope: &ScopeSummary, ceiling: &ScopeSummary) -> bool {
    scope.read.is_subset(&ceiling.read)
        && scope.write.is_subset(&ceiling.write)
        && scope.read_sensitive.is_subset(&ceiling.read_sensitive)
        && scope.relations.is_subset(&ceiling.relations)
        && (!scope.instance_all || ceiling.instance_all)
}

/// Apply a restore (re-widen) to the profile at `path`, the reverse of
/// [`revoke_at`] and the one authority-growth path in the system. Load the
/// profile, re-derive its scopes, re-widen the document in place, then prove the
/// result BOTH (1) strictly widened the original ([`is_strict_widening`]) AND
/// (2) stays within the app's declared `ceiling` ([`is_within_ceiling`]) - only
/// then write it atomically. Writes nothing unless both proofs pass, so a no-op,
/// a non-widening edit, or a reach outside the declared ceiling leaves the file
/// untouched (`NoChange` / `NotPermitted`).
///
/// `ceiling` is the app's declared capability ceiling (the Grant node's
/// `declared_ceiling`, re-derived into a [`ScopeSummary`] by the caller). It is
/// the load-bearing safety bound: a restore may reinstate a capability the app
/// already declared and the user just removed, never grant one it never had. The
/// caller is responsible for the caller-auth gate (the `settings` principal) and
/// for supplying the ceiling from the authoritative grant, exactly as `revoke_at`'s
/// caller owns the auth + tier gate.
pub fn restore_at(
    path: &Path,
    reach: &RevokedReach,
    ceiling: &ScopeSummary,
) -> std::io::Result<RestoreOutcome> {
    if !path.exists() {
        return Ok(RestoreOutcome::NotFound);
    }
    let old_text = std::fs::read_to_string(path)?;
    let old_profile: PermissionProfile = toml::from_str(&old_text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let old_summary = summarize(&old_profile);

    let mut doc: toml_edit::DocumentMut = old_text
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if !apply_restore(&mut doc, reach) {
        // The reach was already present.
        return Ok(RestoreOutcome::NoChange);
    }
    let new_text = doc.to_string();

    let new_profile: PermissionProfile = toml::from_str(&new_text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let new_summary = summarize(&new_profile);

    // Gate 1: the edit only grew authority (no dimension shrank). Gate 2 (the
    // load-bearing one): the widened result stays within the declared ceiling.
    // Both must pass; otherwise write nothing.
    if !is_strict_widening(&old_summary, &new_summary)
        || !is_within_ceiling(&new_summary, ceiling)
    {
        return Ok(RestoreOutcome::NotPermitted);
    }

    atomic_write(path, new_text.as_bytes())?;
    Ok(RestoreOutcome::Restored)
}

/// The per-app record of reaches the user has revoked and not yet restored: the
/// authoritative bound on what a restore may re-add (living-capability-graph.md
/// §6, the way-back).
///
/// A restore is the ONE authority-growth path, so it must be bounded to "exactly
/// what the user removed". Neither of the obvious ceilings supplies that: the
/// Grant node's `declared_ceiling` NARROWS on re-mint (after a revoke the app
/// reconnects and the token is minted from the narrowed profile, so the ceiling no
/// longer contains the revoked reach), and the root-owned SYSTEM-tier profile
/// cannot serve as the immutable original either, because "system-base-wins"
/// ignores the user overlay a revoke writes - so revoke only ever functions on
/// user-tier-only apps, which have no system-tier original. What remains, and the
/// only sound source, is to RECORD each removal when it happens: `revoke` records
/// the reach here, `restore` may re-add a reach ONLY if it is recorded here, and
/// drops it on restore. This is the pure state core; persistence (alongside the
/// profile) and the revoke/restore wiring are the caller's, kept out so the gate
/// logic is unit-tested without I/O.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemovalLedger {
    /// The reaches the user revoked and has not yet restored. A `Vec` (not a set):
    /// `RevokedReach` is `Eq` but the closed variants make membership a linear
    /// scan, and a per-app ledger is tiny.
    removed: Vec<RevokedReach>,
}

impl RemovalLedger {
    /// Record a revoked reach (idempotent - a reach already recorded is not
    /// duplicated). The caller invokes this after a revoke that actually narrowed.
    pub fn record(&mut self, reach: RevokedReach) {
        if !self.removed.contains(&reach) {
            self.removed.push(reach);
        }
    }

    /// Whether this reach is a recorded removal - the restore gate: a restore may
    /// re-add a reach only if this is true.
    pub fn contains(&self, reach: &RevokedReach) -> bool {
        self.removed.contains(reach)
    }

    /// Drop a reach from the ledger, called after a successful restore (it is no
    /// longer "removed"). Idempotent.
    pub fn clear(&mut self, reach: &RevokedReach) {
        self.removed.retain(|r| r != reach);
    }

    /// The recorded removals, for the App-access "Recently removed" surface.
    pub fn removed(&self) -> &[RevokedReach] {
        &self.removed
    }

    /// Whether nothing is currently recorded as removed.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty()
    }
}

/// Write `bytes` to `path` atomically: a sibling temp file, fsync, rename over
/// the target, then fsync the directory so the rename is durable. A crash leaves
/// either the old profile or the new, never a torn file.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "profile path has no parent")
    })?;
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(read: &[&str], write: &[&str], instance_all: bool) -> ScopeSummary {
        ScopeSummary {
            read: read.iter().map(|s| s.to_string()).collect(),
            write: write.iter().map(|s| s.to_string()).collect(),
            read_sensitive: BTreeSet::new(),
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
    fn the_gate_covers_read_sensitive() {
        // Dropping a read_sensitive entry (and nothing else) is a narrowing the
        // gate must see, and gaining one is a widening it must refuse.
        let mut old = summary(&["system.File"], &[], false);
        old.read_sensitive.insert("system.File.secret".into());
        let new = summary(&["system.File"], &[], false); // read_sensitive empty
        assert!(is_strict_narrowing(&old, &new), "dropping read_sensitive is narrowing");
        assert!(!is_strict_narrowing(&new, &old), "gaining read_sensitive is a widening");
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

    const SAMPLE: &str = r#"# my app profile
[info]
app_id = "com.test"

[graph]
read = ["system.File", "system.Project"]
write = ["com.test.Note"]
relations = [
    { from = "com.test.Note", to = "system.File", type = "REFERENCES" },
    { from = "system.File", to = "system.Project", type = "FILE_PART_OF" },
]
instance_scope = "all"
"#;

    fn doc(s: &str) -> toml_edit::DocumentMut {
        s.parse().unwrap()
    }

    fn profile(s: &str) -> PermissionProfile {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn revoke_read_removes_only_that_pattern_and_preserves_the_rest() {
        let mut d = doc(SAMPLE);
        let changed = apply_revoke(
            &mut d,
            &RevokedReach::Read {
                entity_pattern: "system.Project".into(),
            },
        );
        assert!(changed);
        let out = d.to_string();
        // The read array now holds only the kept pattern (checked on the read
        // line, since "system.Project" also legitimately appears as a relation
        // endpoint and must NOT be touched there).
        let read_line = out.lines().find(|l| l.trim_start().starts_with("read =")).unwrap();
        assert!(read_line.contains("system.File"), "the kept read pattern survives");
        assert!(!read_line.contains("system.Project"), "the revoked read pattern is gone");
        // The relation endpoint of the same name is untouched.
        assert!(out.contains("to = \"system.Project\""), "relations are not touched");
        // Format-preserving: the comment and other sections are untouched.
        assert!(out.contains("# my app profile"));
        assert!(out.contains("write = [\"com.test.Note\"]"));
        assert!(out.contains("instance_scope = \"all\""));
    }

    #[test]
    fn revoke_relation_removes_only_the_matching_entry() {
        let mut d = doc(SAMPLE);
        let changed = apply_revoke(
            &mut d,
            &RevokedReach::Relation {
                from: "com.test.Note".into(),
                to: "system.File".into(),
                relation_type: "REFERENCES".into(),
            },
        );
        assert!(changed);
        let out = d.to_string();
        assert!(!out.contains("REFERENCES"), "the revoked relation is gone");
        assert!(out.contains("FILE_PART_OF"), "the other relation survives");
    }

    #[test]
    fn revoke_relation_handles_the_array_of_tables_form() {
        // serde's TOML serializer emits a Vec<struct> as [[graph.relations]],
        // not an inline array, so apply_revoke must handle that form too.
        let mut d = doc(
            "[graph]\nread = [\"system.File\"]\n\
             [[graph.relations]]\nfrom = \"com.test.Note\"\nto = \"system.File\"\ntype = \"REFERENCES\"\n\
             [[graph.relations]]\nfrom = \"system.File\"\nto = \"system.Project\"\ntype = \"FILE_PART_OF\"\n",
        );
        let changed = apply_revoke(
            &mut d,
            &RevokedReach::Relation {
                from: "com.test.Note".into(),
                to: "system.File".into(),
                relation_type: "REFERENCES".into(),
            },
        );
        assert!(changed, "the array-of-tables relation is removed");
        let out = d.to_string();
        assert!(!out.contains("REFERENCES"), "the revoked relation is gone");
        assert!(out.contains("FILE_PART_OF"), "the other relation survives");
    }

    #[test]
    fn revoke_instance_all_demotes_to_own() {
        let mut d = doc(SAMPLE);
        let changed = apply_revoke(&mut d, &RevokedReach::InstanceAll);
        assert!(changed);
        assert!(d.to_string().contains("instance_scope = \"own\""));
    }

    #[test]
    fn revoking_an_absent_reach_is_a_no_op() {
        let mut d = doc(SAMPLE);
        // A read pattern that is not present.
        assert!(!apply_revoke(
            &mut d,
            &RevokedReach::Read { entity_pattern: "system.Event".into() }
        ));
        // instance already own after a first demote -> a second demote is a no-op.
        apply_revoke(&mut d, &RevokedReach::InstanceAll);
        assert!(!apply_revoke(&mut d, &RevokedReach::InstanceAll), "already own -> no-op");
    }

    #[test]
    fn revoke_on_a_profile_without_a_graph_table_is_a_no_op() {
        let mut d = doc("[info]\napp_id = \"com.x\"\n");
        assert!(!apply_revoke(
            &mut d,
            &RevokedReach::Read { entity_pattern: "system.File".into() }
        ));
    }

    #[test]
    fn revoke_at_narrows_the_file_when_the_gate_passes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("com.test.toml");
        std::fs::write(&path, SAMPLE).unwrap();

        let outcome = revoke_at(
            &path,
            &RevokedReach::Read { entity_pattern: "system.Project".into() },
        )
        .unwrap();
        assert_eq!(outcome, RevokeOutcome::Revoked);

        // The file now reflects the narrowed read set, format preserved.
        let after = std::fs::read_to_string(&path).unwrap();
        let read_line = after.lines().find(|l| l.trim_start().starts_with("read =")).unwrap();
        assert!(!read_line.contains("system.Project"), "read narrowed on disk");
        assert!(after.contains("# my app profile"), "comments preserved");
    }

    #[test]
    fn revoke_at_writes_nothing_for_an_absent_reach() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("com.test.toml");
        std::fs::write(&path, SAMPLE).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let outcome = revoke_at(
            &path,
            &RevokedReach::Read { entity_pattern: "system.NotPresent".into() },
        )
        .unwrap();
        assert_eq!(outcome, RevokeOutcome::NoChange);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "file untouched");
    }

    #[test]
    fn revoke_at_reports_not_found_for_a_missing_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("absent.toml");
        let outcome = revoke_at(
            &path,
            &RevokedReach::InstanceAll,
        )
        .unwrap();
        assert_eq!(outcome, RevokeOutcome::NotFound);
    }

    #[test]
    fn from_profile_summarises_raw_graph_entries() {
        let profile: PermissionProfile = toml::from_str(
            "[graph]\nread = [\"system.File.path\", \"system.Project\"]\n\
             write = [\"com.x.Note\"]\n\
             relations = [{ from = \"system.File\", to = \"system.Project\", type = \"FILE_PART_OF\" }]\n\
             instance_scope = \"all\"\n",
        )
        .unwrap();
        let s = ScopeSummary::from_profile(&profile);
        // Raw patterns, not collapsed entity types: the field-scoped entry is kept verbatim.
        assert!(s.read.contains("system.File.path"));
        assert!(s.read.contains("system.Project"));
        assert!(s.write.contains("com.x.Note"));
        assert!(s.relations.contains(&(
            "system.File".to_string(),
            "system.Project".to_string(),
            "FILE_PART_OF".to_string()
        )));
        assert!(s.instance_all);
    }

    #[test]
    fn revoking_one_field_pattern_while_another_for_the_same_type_remains_is_narrowing() {
        // Regression: with the old entity-type-collapsed summary, removing
        // `system.File.path` while `system.File.name` stayed left the entity-type
        // set unchanged and the gate false-refused the revoke. Raw-pattern
        // comparison sees the genuine narrowing.
        let parse = |t: &str| -> PermissionProfile { toml::from_str(t).unwrap() };
        let old = parse(
            "[graph]\nread = [\"system.File.path\", \"system.File.name\"]\ninstance_scope = \"own\"\n",
        );
        let new = parse("[graph]\nread = [\"system.File.name\"]\ninstance_scope = \"own\"\n");
        assert!(
            is_strict_narrowing(&ScopeSummary::from_profile(&old), &ScopeSummary::from_profile(&new)),
            "dropping a field-scoped read pattern is a real narrowing"
        );
    }

    // ── Restore (re-widen) pure core ──

    #[test]
    fn adding_a_read_type_is_strict_widening() {
        let old = summary(&["system.File"], &[], false);
        let new = summary(&["system.File", "system.Project"], &[], false);
        assert!(is_strict_widening(&old, &new));
        // The reverse (a narrowing) is not a widening.
        assert!(!is_strict_widening(&new, &old));
    }

    #[test]
    fn a_noop_is_not_a_widening() {
        let s = summary(&["system.File"], &[], false);
        assert!(!is_strict_widening(&s, &s.clone()), "a no-op must be refused");
    }

    #[test]
    fn gaining_instance_all_is_a_widening() {
        let old = summary(&["system.File"], &[], false);
        let new = summary(&["system.File"], &[], true);
        assert!(is_strict_widening(&old, &new));
        // Losing `All` is a narrowing, not a widening.
        assert!(!is_strict_widening(&new, &old));
    }

    #[test]
    fn widening_one_dimension_while_narrowing_another_is_refused() {
        // Added a read but dropped a write: not a clean superset (write shrank), so
        // the gate refuses it - a restore must ONLY grow.
        let old = summary(&["system.File"], &["com.x.Note"], false);
        let new = summary(&["system.File", "system.Project"], &[], false);
        assert!(!is_strict_widening(&old, &new));
    }

    #[test]
    fn apply_restore_re_adds_a_read_type_idempotently() {
        let mut d = doc(SAMPLE);
        let added = apply_restore(
            &mut d,
            &RevokedReach::Read { entity_pattern: "system.Session".into() },
        );
        assert!(added);
        let read_line = d
            .to_string()
            .lines()
            .find(|l| l.trim_start().starts_with("read ="))
            .unwrap()
            .to_string();
        assert!(read_line.contains("system.Session"), "the restored read pattern is present");
        assert!(read_line.contains("system.File"), "the existing reads are preserved");
        // Idempotent: re-adding a present pattern adds nothing.
        assert!(!apply_restore(
            &mut d,
            &RevokedReach::Read { entity_pattern: "system.Session".into() }
        ));
    }

    #[test]
    fn apply_restore_re_creates_a_removed_read_array() {
        // A profile a revoke had stripped to no `read` array: restore re-creates it.
        let mut d = doc("[graph]\nwrite = [\"com.x.Note\"]\n");
        assert!(apply_restore(
            &mut d,
            &RevokedReach::Read { entity_pattern: "system.File".into() }
        ));
        assert!(d.to_string().contains("system.File"));
    }

    #[test]
    fn apply_restore_promotes_instance_to_all_idempotently() {
        let mut d = doc("[graph]\ninstance_scope = \"own\"\n");
        assert!(apply_restore(&mut d, &RevokedReach::InstanceAll));
        assert!(d.to_string().contains("instance_scope = \"all\""));
        assert!(!apply_restore(&mut d, &RevokedReach::InstanceAll), "already all is a no-op");
    }

    #[test]
    fn restore_is_the_inverse_of_revoke() {
        // Revoke a read type, then restore it: back to the original read set. The
        // property the panel's Undo / way-back relies on.
        let mut d = doc(SAMPLE);
        let reach = RevokedReach::Read { entity_pattern: "system.Project".into() };
        let read_of = |d: &toml_edit::DocumentMut| {
            d.to_string()
                .lines()
                .find(|l| l.trim_start().starts_with("read ="))
                .unwrap()
                .to_string()
        };
        assert!(apply_revoke(&mut d, &reach));
        assert!(!read_of(&d).contains("system.Project"), "revoked out");
        assert!(apply_restore(&mut d, &reach));
        assert!(read_of(&d).contains("system.Project"), "restored back");
    }

    #[test]
    fn is_within_ceiling_bounds_every_dimension() {
        let ceiling = summary(&["system.File", "system.Project"], &["com.x.Note"], true);
        // A subset in every dimension is within the ceiling.
        assert!(is_within_ceiling(&summary(&["system.File"], &[], false), &ceiling));
        // A read outside the declared ceiling is not.
        assert!(!is_within_ceiling(&summary(&["system.Grant"], &[], false), &ceiling));
        // Instance `All` is allowed when the ceiling permits it, refused otherwise.
        assert!(is_within_ceiling(&summary(&["system.File"], &[], true), &ceiling));
        let own_ceiling = summary(&["system.File"], &[], false);
        assert!(!is_within_ceiling(&summary(&["system.File"], &[], true), &own_ceiling));
    }

    #[test]
    fn restore_at_re_widens_within_ceiling() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("com.test.toml");
        std::fs::write(&path, SAMPLE).unwrap();
        // Ceiling = SAMPLE's declared scope plus system.Session in read (declared by
        // the app, previously removed by the user).
        let ceiling_profile = profile(&SAMPLE.replace(
            "read = [\"system.File\", \"system.Project\"]",
            "read = [\"system.File\", \"system.Project\", \"system.Session\"]",
        ));
        let ceiling = ScopeSummary::from_profile(&ceiling_profile);
        let outcome = restore_at(
            &path,
            &RevokedReach::Read { entity_pattern: "system.Session".into() },
            &ceiling,
        )
        .unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        let after = std::fs::read_to_string(&path).unwrap();
        let read_line = after.lines().find(|l| l.trim_start().starts_with("read =")).unwrap();
        assert!(read_line.contains("system.Session"), "restored on disk");
        assert!(after.contains("# my app profile"), "comments preserved");
    }

    #[test]
    fn restore_at_refuses_a_reach_outside_the_ceiling() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("com.test.toml");
        std::fs::write(&path, SAMPLE).unwrap();
        // Ceiling = exactly SAMPLE's declared scope: system.Grant is NOT in it.
        let ceiling = ScopeSummary::from_profile(&profile(SAMPLE));
        let outcome = restore_at(
            &path,
            &RevokedReach::Read { entity_pattern: "system.Grant".into() },
            &ceiling,
        )
        .unwrap();
        assert_eq!(outcome, RestoreOutcome::NotPermitted);
        // The file is untouched: a reach outside the declared ceiling is not written.
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("system.Grant"), "a never-declared reach is refused");
    }

    #[test]
    fn restore_at_no_change_for_a_present_reach() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("com.test.toml");
        std::fs::write(&path, SAMPLE).unwrap();
        let ceiling = ScopeSummary::from_profile(&profile(SAMPLE));
        let outcome = restore_at(
            &path,
            &RevokedReach::Read { entity_pattern: "system.File".into() },
            &ceiling,
        )
        .unwrap();
        assert_eq!(outcome, RestoreOutcome::NoChange);
    }

    #[test]
    fn restore_at_reports_not_found_for_a_missing_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("nope.toml");
        let ceiling = ScopeSummary::from_profile(&profile(SAMPLE));
        let outcome = restore_at(
            &path,
            &RevokedReach::Read { entity_pattern: "system.File".into() },
            &ceiling,
        )
        .unwrap();
        assert_eq!(outcome, RestoreOutcome::NotFound);
    }

    #[test]
    fn removal_ledger_gates_restore_to_recorded_removals() {
        let mut ledger = RemovalLedger::default();
        assert!(ledger.is_empty());
        let read = RevokedReach::Read { entity_pattern: "system.File".into() };
        let write = RevokedReach::Write { entity_pattern: "com.x.Note".into() };

        // A never-removed reach is not restorable.
        assert!(!ledger.contains(&read), "nothing recorded yet");

        // Recording a removal makes it restorable; idempotent.
        ledger.record(read.clone());
        ledger.record(read.clone());
        assert!(ledger.contains(&read));
        assert_eq!(ledger.removed().len(), 1, "no duplicate");
        // A different, unrecorded reach stays non-restorable.
        assert!(!ledger.contains(&write), "only recorded removals are restorable");

        // Clearing (after a restore) drops it; idempotent.
        ledger.clear(&read);
        ledger.clear(&read);
        assert!(!ledger.contains(&read));
        assert!(ledger.is_empty());
    }

    #[test]
    fn removal_ledger_round_trips_as_toml() {
        let mut ledger = RemovalLedger::default();
        ledger.record(RevokedReach::Read { entity_pattern: "system.File".into() });
        ledger.record(RevokedReach::InstanceAll);
        let text = toml::to_string(&ledger).unwrap();
        let back: RemovalLedger = toml::from_str(&text).unwrap();
        assert_eq!(back.removed().len(), 2);
        assert!(back.contains(&RevokedReach::InstanceAll));
    }
}
