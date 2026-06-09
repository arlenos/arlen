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
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::permission::PermissionProfile;
use crate::quota::{AppTier, QuotaConfig};
use crate::token::InstanceScope;

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
    /// The raw `[graph].read` pattern entries (e.g. `system.File`,
    /// `system.File.path`, `com.x.*`). Kept as the verbatim patterns, NOT
    /// collapsed to entity types: a field-scoped pattern (`system.File.path`)
    /// and a broader one (`system.File`) are distinct entries, so removing one
    /// while another for the same type remains is a real narrowing the gate must
    /// see. Collapsing to entity types would hide it and false-refuse the revoke.
    pub read: BTreeSet<String>,
    /// The raw `[graph].write` pattern entries, same discipline as `read`.
    pub write: BTreeSet<String>,
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
        let (read, write) = match &profile.graph {
            Some(g) => (
                g.read.iter().cloned().collect(),
                g.write.iter().cloned().collect(),
            ),
            None => (BTreeSet::new(), BTreeSet::new()),
        };
        ScopeSummary {
            read,
            write,
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
            let Some(arr) = graph.get_mut("relations").and_then(Item::as_array_mut) else {
                return false;
            };
            let before = arr.len();
            arr.retain(|v| {
                // Keep every entry that does NOT match all three keys.
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

/// Remove the first array element equal to `value` from the `key` string array of
/// `graph`. Returns whether an element was removed.
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

/// The outcome of a revoke attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevokeOutcome {
    /// The profile was narrowed and written.
    Revoked,
    /// The reach was already absent; nothing changed, nothing written.
    NoChange,
    /// The narrowing did not strictly shrink authority (the safety gate refused);
    /// nothing written.
    NotNarrowing,
    /// No profile file exists for the app; nothing to narrow.
    NotFound,
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
}
