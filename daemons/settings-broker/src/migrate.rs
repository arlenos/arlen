//! PAS-5: carrying a user's value across a renamed key.
//!
//! A schema declares `renamed_from` on the new key, so a rename is expressed
//! once, by the app that made it. The broker forwards at LOAD, before any reader
//! sees the config, so nothing downstream has to know the key ever had another
//! name.
//!
//! **Only an explicitly-set value is forwarded.** That is Chromium's
//! `GetUserPrefValue()` rule, and it matters more here because we have a layered
//! resolver: an unset key still READS as a value (the schema's default), so
//! forwarding "whatever the key currently resolves to" would silently promote a
//! default into a user setting - freezing today's default forever and making the
//! user's config diverge from the app's intent on the next update.
//!
//! **No transform language.** A value that needs reshaping is a NEW key plus the
//! old one removed, and the app converts it in its own code. A declarative
//! transform DSL is the kind of thing that grows until it needs a debugger.
//!
//! **Unknown keys are parked, never deleted.** A key the schema does not declare
//! may belong to a plugin, a newer version, or a hand edit the user meant. The
//! plan is explicit that orphans are surfaced rather than removed, so this
//! reports them and touches nothing.

use arlen_forage_recipe::settings::SettingsSchema;
use toml_edit::DocumentMut;

/// One value to carry across a rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    /// The old key the value sits under today.
    pub from: String,
    /// The key it belongs under now.
    pub to: String,
}

/// What a migration would do. Computing this separately from applying it means
/// the decision is testable, and a caller can show the user what will happen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MigrationPlan {
    /// Values to carry across a rename.
    pub forwards: Vec<Forward>,
    /// Old keys to drop because the user has ALREADY set the new name, so the
    /// old value is stale rather than the one to keep.
    pub supersedes: Vec<String>,
    /// Keys present in the config that the schema does not declare. Reported so
    /// they can be shown; never modified.
    pub orphans: Vec<String>,
}

impl MigrationPlan {
    /// Whether the plan would change anything.
    pub fn is_empty(&self) -> bool {
        self.forwards.is_empty() && self.supersedes.is_empty()
    }
}

/// Whether the document has an explicitly-set value at a dotted key.
fn is_set(doc: &DocumentMut, key: &str) -> bool {
    let mut item = doc.as_item();
    for segment in key.split('.') {
        match item.get(segment) {
            Some(next) => item = next,
            None => return false,
        }
    }
    item.is_value()
}

/// Read the value at a dotted key.
fn value_at(doc: &DocumentMut, key: &str) -> Option<toml_edit::Value> {
    let mut item = doc.as_item();
    for segment in key.split('.') {
        item = item.get(segment)?;
    }
    item.as_value().cloned()
}

/// Remove a dotted key, leaving any tables in place.
fn remove_key(doc: &mut DocumentMut, key: &str) {
    let segments: Vec<&str> = key.split('.').collect();
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut table = doc.as_table_mut();
    for segment in parents {
        match table.get_mut(segment).and_then(|i| i.as_table_mut()) {
            Some(next) => table = next,
            None => return,
        }
    }
    table.remove(last);
}

/// Work out what migrating this config against this schema would do.
pub fn plan_migration(schema: &SettingsSchema, doc: &DocumentMut) -> MigrationPlan {
    let mut plan = MigrationPlan::default();

    for item in schema.sections.iter().flat_map(|s| s.items.iter()) {
        for old in &item.renamed_from {
            if !is_set(doc, old) {
                continue;
            }
            if is_set(doc, &item.key) {
                // The user has set the new name too, so their current choice
                // wins and the old value is stale.
                plan.supersedes.push(old.clone());
            } else {
                plan.forwards.push(Forward {
                    from: old.clone(),
                    to: item.key.clone(),
                });
            }
        }
    }

    plan.orphans = orphans(schema, doc);
    plan
}

/// Top-level keys the schema neither declares nor names as a former name.
///
/// Deliberately shallow: descending into tables would report an app's own
/// nested state as orphaned, and the point is to surface keys the user might
/// care about, not to audit every leaf.
fn orphans(schema: &SettingsSchema, doc: &DocumentMut) -> Vec<String> {
    let mut known: Vec<&str> = Vec::new();
    for item in schema.sections.iter().flat_map(|s| s.items.iter()) {
        known.push(&item.key);
        for old in &item.renamed_from {
            known.push(old);
        }
    }

    doc.as_table()
        .iter()
        .filter(|(name, item)| {
            item.is_value()
                && !known
                    .iter()
                    .any(|k| *k == *name || k.split('.').next() == Some(name))
        })
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Apply a plan to the document. Returns whether anything changed.
///
/// Idempotent: forwarding removes the old key, so a second run finds nothing to
/// do. That matters because the broker migrates at every load, and a migration
/// that re-fired would keep overwriting whatever the user set since.
pub fn apply_migration(doc: &mut DocumentMut, plan: &MigrationPlan) -> bool {
    let mut changed = false;

    for forward in &plan.forwards {
        let Some(value) = value_at(doc, &forward.from) else {
            continue;
        };
        if crate::apply::set_in_document(doc, &forward.to, &to_toml(&value)).unwrap_or(false) {
            changed = true;
        }
        remove_key(doc, &forward.from);
        changed = true;
    }

    for stale in &plan.supersedes {
        remove_key(doc, stale);
        changed = true;
    }

    changed
}

/// Convert an editable value back to the plain one the apply path takes.
///
/// Converted variant by variant rather than via text: a `toml_edit` value
/// renders WITH its decor (` "dark"`, leading space and quotes included) and
/// `toml::Value` parses documents rather than bare scalars, so a text round trip
/// silently produced a string containing the quotes instead of the value.
fn to_toml(value: &toml_edit::Value) -> toml::Value {
    match value {
        toml_edit::Value::String(s) => toml::Value::String(s.value().clone()),
        toml_edit::Value::Integer(i) => toml::Value::Integer(*i.value()),
        toml_edit::Value::Float(f) => toml::Value::Float(*f.value()),
        toml_edit::Value::Boolean(b) => toml::Value::Boolean(*b.value()),
        toml_edit::Value::Datetime(d) => toml::Value::String(d.value().to_string()),
        toml_edit::Value::Array(a) => toml::Value::Array(a.iter().map(to_toml).collect()),
        toml_edit::Value::InlineTable(t) => toml::Value::Table(
            t.iter()
                .map(|(k, v)| (k.to_string(), to_toml(v)))
                .collect(),
        ),
    }
}

/// Migrate the config at `path` against `schema`, returning the keys whose
/// value moved.
///
/// Called before any write is applied, which is what "the broker runs
/// migrations at load, before any reader" means in practice: by the time a
/// caller's write lands, or the app next reads the file, the rename has already
/// been carried across.
///
/// A missing or unreadable file is not an error - there is simply nothing to
/// migrate. A MALFORMED file is left strictly alone: rewriting a config we
/// cannot parse would risk destroying whatever it really contains, and the
/// write path refuses it separately with a clearer message.
pub fn migrate_file(
    path: &std::path::Path,
    schema: &SettingsSchema,
) -> Result<Vec<String>, crate::apply::ApplyError> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let Ok(mut doc) = text.parse::<DocumentMut>() else {
        return Ok(Vec::new());
    };

    let plan = plan_migration(schema, &doc);
    if plan.is_empty() {
        return Ok(Vec::new());
    }
    if !apply_migration(&mut doc, &plan) {
        return Ok(Vec::new());
    }

    crate::apply::write_document(path, &doc)?;
    Ok(plan.forwards.iter().map(|f| f.to.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::settings::{
        SettingScope, SettingType, SettingsItem, SettingsSection,
    };

    fn item(key: &str, renamed_from: &[&str]) -> SettingsItem {
        SettingsItem {
            key: key.into(),
            value_type: SettingType::String,
            label: "L".into(),
            description: None,
            default: None,
            min: None,
            max: None,
            unit: None,
            options: Vec::new(),
            order: None,
            keywords: Vec::new(),
            scope: SettingScope::default(),
            tags: Vec::new(),
            included: None,
            deprecated_message: None,
            replaced_by: None,
            renamed_from: renamed_from.iter().map(|s| s.to_string()).collect(),
            since: None,
            removed_in: None,
            visible_when: None,
        }
    }

    fn schema(items: Vec<SettingsItem>) -> SettingsSchema {
        SettingsSchema {
            version: 2,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items,
            }],
        }
    }

    fn doc(text: &str) -> DocumentMut {
        text.parse::<DocumentMut>().unwrap()
    }

    #[test]
    fn a_user_set_old_key_is_forwarded() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let mut d = doc("theme = \"dark\"\n");

        let plan = plan_migration(&schema, &d);
        assert_eq!(
            plan.forwards,
            vec![Forward {
                from: "theme".into(),
                to: "colour_scheme".into()
            }]
        );

        assert!(apply_migration(&mut d, &plan));
        assert!(is_set(&d, "colour_scheme"));
        assert!(!is_set(&d, "theme"));
        assert_eq!(
            value_at(&d, "colour_scheme").unwrap().as_str(),
            Some("dark")
        );
    }

    /// The broker migrates at every load, so a re-fire would keep clobbering
    /// whatever the user set since.
    #[test]
    fn migrating_twice_changes_nothing_the_second_time() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let mut d = doc("theme = \"dark\"\n");

        let plan = plan_migration(&schema, &d);
        apply_migration(&mut d, &plan);

        let second = plan_migration(&schema, &d);
        assert!(second.is_empty(), "{second:?}");
        assert!(!apply_migration(&mut d, &second));
    }

    /// An unset key still READS as a value through the schema default, so
    /// forwarding a resolved value would promote a default into a user setting
    /// and freeze it forever.
    #[test]
    fn an_unset_old_key_is_not_forwarded() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let d = doc("unrelated = 1\n");
        assert!(plan_migration(&schema, &d).is_empty());
    }

    /// When the user has set the new name, their current choice wins and the
    /// old value is dropped as stale rather than overwriting it.
    #[test]
    fn an_explicit_new_value_supersedes_the_old_one() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let mut d = doc("theme = \"dark\"\ncolour_scheme = \"light\"\n");

        let plan = plan_migration(&schema, &d);
        assert!(plan.forwards.is_empty());
        assert_eq!(plan.supersedes, vec!["theme".to_string()]);

        apply_migration(&mut d, &plan);
        assert_eq!(
            value_at(&d, "colour_scheme").unwrap().as_str(),
            Some("light"),
            "the user's current choice must survive"
        );
        assert!(!is_set(&d, "theme"));
    }

    /// A key may have been renamed more than once, so every former name is
    /// checked.
    #[test]
    fn any_of_several_former_names_forwards() {
        let schema = schema(vec![item("colour_scheme", &["theme", "skin"])]);
        let mut d = doc("skin = \"dark\"\n");

        let plan = plan_migration(&schema, &d);
        assert_eq!(plan.forwards.len(), 1);
        assert_eq!(plan.forwards[0].from, "skin");
        apply_migration(&mut d, &plan);
        assert_eq!(
            value_at(&d, "colour_scheme").unwrap().as_str(),
            Some("dark")
        );
    }

    /// Unknown keys are surfaced but never touched: they may belong to a plugin,
    /// a newer version, or a hand edit the user meant.
    #[test]
    fn an_unknown_key_is_reported_and_left_alone() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let mut d = doc("colour_scheme = \"dark\"\nmystery = 42\n");

        let plan = plan_migration(&schema, &d);
        assert_eq!(plan.orphans, vec!["mystery".to_string()]);
        assert!(plan.is_empty(), "an orphan alone is not a change");

        apply_migration(&mut d, &plan);
        assert!(is_set(&d, "mystery"), "an orphan must never be deleted");
    }

    /// A former name is not an orphan: it is known, and about to be forwarded.
    #[test]
    fn a_former_name_is_not_reported_as_an_orphan() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let d = doc("theme = \"dark\"\n");
        assert!(plan_migration(&schema, &d).orphans.is_empty());
    }

    #[test]
    fn a_dotted_rename_forwards_across_tables() {
        let schema = schema(vec![item("window.width", &["width"])]);
        let mut d = doc("width = 1200\n");

        let plan = plan_migration(&schema, &d);
        apply_migration(&mut d, &plan);
        assert_eq!(value_at(&d, "window.width").unwrap().as_integer(), Some(1200));
        assert!(!is_set(&d, "width"));
    }

    /// Migrating must not destroy the user's comments on keys it does not touch.
    #[test]
    fn migration_preserves_the_rest_of_the_file() {
        let schema = schema(vec![item("colour_scheme", &["theme"])]);
        let mut d = doc("# keep me\nkept = 1\ntheme = \"dark\"\n");

        let plan = plan_migration(&schema, &d);
        apply_migration(&mut d, &plan);

        let text = d.to_string();
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("kept = 1"), "{text}");
    }
}
