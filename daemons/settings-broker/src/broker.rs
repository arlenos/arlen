//! Composing a settings write: decide, apply, and announce exactly what changed.
//!
//! The change signal is the reason the broker exists rather than everyone just
//! writing files. Per the plan it carries the app-id plus **the exact changed
//! key set**, which is what lets an app live-reload without restarting and
//! without polling. GSettings batches a `change-event` that fans out into
//! per-key `changed`; the same shape applies here.
//!
//! Because a subscriber acts on that set, it has to be true. Two rules keep it
//! honest: a key whose value did not actually change never appears in it, and a
//! key that was refused never appears in it either.

use std::path::Path;

use arlen_forage_recipe::settings::SettingsSchema;

use crate::apply::{apply_to_file, ApplyError};
use crate::decide::{decide_write, WriteRejection, WriteRequest};

/// What the broker announces after a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSignal {
    /// The app whose settings changed.
    pub app_id: String,
    /// Exactly the keys whose stored value is now different, in the order they
    /// were written. Empty when nothing changed, in which case there is nothing
    /// to announce.
    pub changed: Vec<String>,
}

impl ChangeSignal {
    /// Whether anything actually changed. An empty signal should not be emitted.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }
}

/// Why a batch did not go through.
#[derive(Debug)]
pub enum BrokerError {
    /// A write was refused by the schema. Carries the offending key so the
    /// caller can say which one, rather than failing the batch anonymously.
    Refused {
        /// The key that was refused.
        key: String,
        /// The rule it violated.
        rejection: WriteRejection,
    },
    /// The batch mixes apps. One call writes one app's settings, so the change
    /// signal has a single unambiguous subject.
    MixedApps,
    /// Applying failed after validation passed.
    Apply(ApplyError),
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrokerError::Refused { key, rejection } => write!(f, "'{key}' refused: {rejection}"),
            BrokerError::MixedApps => write!(f, "a batch must write a single app's settings"),
            BrokerError::Apply(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BrokerError {}

impl From<ApplyError> for BrokerError {
    fn from(e: ApplyError) -> Self {
        BrokerError::Apply(e)
    }
}

/// Validate every write in the batch, then apply them, returning the keys that
/// actually changed.
///
/// **Validation is all-or-nothing, deliberately.** Every request is checked
/// before any is applied, so a batch containing one bad key changes nothing at
/// all. Applying the good half and reporting the rest as refused would leave the
/// user's settings in a state they never asked for and did not see.
///
/// Applying is sequential per key. A filesystem failure partway through leaves
/// the earlier keys written - that is unavoidable without a journal - so the
/// error path still reflects reality rather than claiming nothing happened.
pub fn apply_writes(
    schema: &SettingsSchema,
    config_path: &Path,
    requests: &[WriteRequest],
) -> Result<ChangeSignal, BrokerError> {
    let Some(first) = requests.first() else {
        return Ok(ChangeSignal {
            app_id: String::new(),
            changed: Vec::new(),
        });
    };
    let app_id = first.app_id.clone();
    if requests.iter().any(|r| r.app_id != app_id) {
        return Err(BrokerError::MixedApps);
    }

    // Decide everything first: nothing is written until the whole batch is known
    // to be legal.
    for request in requests {
        decide_write(schema, request).map_err(|rejection| BrokerError::Refused {
            key: request.key.clone(),
            rejection,
        })?;
    }

    // Migrate BEFORE applying: a rename must be carried across before a write
    // lands, or the write would go to the new key while the user's old value
    // still sat under the former name. Forwarded keys are part of the changed
    // set because the file genuinely changed - a subscriber live-reloading needs
    // to pick the moved value up.
    let mut changed = crate::migrate::migrate_file(config_path, schema)?;
    for request in requests {
        if apply_to_file(config_path, &request.key, &request.value)?
            && !changed.contains(&request.key)
        {
            changed.push(request.key.clone());
        }
    }

    Ok(ChangeSignal { app_id, changed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::settings::{
        SettingScope, SettingType, SettingsItem, SettingsSection,
    };
    use toml::Value;

    fn item(key: &str, value_type: SettingType) -> SettingsItem {
        SettingsItem {
            key: key.into(),
            value_type,
            label: "L".into(),
            description: None,
            default: None,
            min: None,
            max: None,
            unit: None,
            options: Vec::new(),
            options_from: None,
            order: None,
            keywords: Vec::new(),
            scope: SettingScope::default(),
            tags: Vec::new(),
            included: None,
            deprecated_message: None,
            replaced_by: None,
            renamed_from: Vec::new(),
            since: None,
            removed_in: None,
            visible_when: None,
        }
    }

    fn schema() -> SettingsSchema {
        SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items: vec![
                    item("theme", SettingType::String),
                    item("count", SettingType::Int),
                    item("enabled", SettingType::Bool),
                ],
            }],
        }
    }

    fn req(key: &str, value: Value) -> WriteRequest {
        WriteRequest {
            app_id: "org.example.App".into(),
            key: key.into(),
            value,
        }
    }

    fn temp_config(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn the_signal_names_exactly_the_written_keys() {
        let (_d, path) = temp_config("");
        let signal = apply_writes(
            &schema(),
            &path,
            &[
                req("theme", Value::String("dark".into())),
                req("count", Value::Integer(3)),
            ],
        )
        .unwrap();
        assert_eq!(signal.app_id, "org.example.App");
        assert_eq!(signal.changed, vec!["theme".to_string(), "count".to_string()]);
    }

    /// A subscriber live-reloads on this set, so a key that did not actually
    /// change must not appear in it.
    #[test]
    fn an_unchanged_key_is_left_out_of_the_signal() {
        let (_d, path) = temp_config("theme = \"dark\"\n");
        let signal = apply_writes(
            &schema(),
            &path,
            &[
                req("theme", Value::String("dark".into())), // same value
                req("count", Value::Integer(7)),            // genuinely new
            ],
        )
        .unwrap();
        assert_eq!(signal.changed, vec!["count".to_string()]);
    }

    #[test]
    fn a_wholly_unchanged_batch_signals_nothing() {
        let (_d, path) = temp_config("theme = \"dark\"\n");
        let signal =
            apply_writes(&schema(), &path, &[req("theme", Value::String("dark".into()))]).unwrap();
        assert!(signal.is_empty());
    }

    /// One bad key must not let the rest through: a partially-applied batch
    /// leaves settings in a state the user never asked for.
    #[test]
    fn a_batch_with_one_refusal_changes_nothing() {
        let (_d, path) = temp_config("");
        let before = std::fs::read_to_string(&path).unwrap();

        let result = apply_writes(
            &schema(),
            &path,
            &[
                req("theme", Value::String("light".into())), // legal
                req("count", Value::String("many".into())),  // wrong type
            ],
        );
        match result {
            Err(BrokerError::Refused { key, .. }) => assert_eq!(key, "count"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        // The legal write must NOT have landed.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn an_undeclared_key_refuses_the_batch() {
        let (_d, path) = temp_config("");
        let result = apply_writes(&schema(), &path, &[req("nope", Value::Boolean(true))]);
        assert!(
            matches!(
                result,
                Err(BrokerError::Refused {
                    rejection: WriteRejection::UndeclaredKey,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    /// The signal has one subject, so a batch cannot span apps.
    #[test]
    fn a_batch_may_not_mix_apps() {
        let (_d, path) = temp_config("");
        let mut other = req("count", Value::Integer(1));
        other.app_id = "org.other.App".into();

        let result = apply_writes(
            &schema(),
            &path,
            &[req("theme", Value::String("x".into())), other],
        );
        assert!(matches!(result, Err(BrokerError::MixedApps)), "{result:?}");
    }

    #[test]
    fn an_empty_batch_is_a_no_op() {
        let (_d, path) = temp_config("");
        let signal = apply_writes(&schema(), &path, &[]).unwrap();
        assert!(signal.is_empty());
    }
    /// A rename must be carried across BEFORE the write lands, or the write goes
    /// to the new key while the user's old value still sits under the former
    /// name - two values for one setting, and the older one wins on next read.
    #[test]
    fn a_pending_rename_is_migrated_before_the_write() {
        let mut renamed = item("colour_scheme", SettingType::String);
        renamed.renamed_from = vec!["theme".into()];
        let schema = SettingsSchema {
            version: 2,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items: vec![renamed, item("count", SettingType::Int)],
            }],
        };

        // The user's value still sits under the OLD name.
        let (_d, path) = temp_config("theme = \"dark\"\n");

        // A write to an unrelated key still triggers the migration.
        let signal = apply_writes(&schema, &path, &[req("count", Value::Integer(3))]).unwrap();

        let parsed: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed["colour_scheme"].as_str(),
            Some("dark"),
            "the value should have moved to the new key"
        );
        assert!(parsed.get("theme").is_none(), "the old key should be gone");

        // The moved key is announced too: a subscriber has to pick it up.
        assert!(
            signal.changed.contains(&"colour_scheme".to_string()),
            "{:?}",
            signal.changed
        );
        assert!(signal.changed.contains(&"count".to_string()));
    }

    /// Migration is idempotent, so a second write does not re-announce it.
    #[test]
    fn a_completed_migration_is_not_re_announced() {
        let mut renamed = item("colour_scheme", SettingType::String);
        renamed.renamed_from = vec!["theme".into()];
        let schema = SettingsSchema {
            version: 2,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items: vec![renamed, item("count", SettingType::Int)],
            }],
        };
        let (_d, path) = temp_config("theme = \"dark\"\n");

        apply_writes(&schema, &path, &[req("count", Value::Integer(1))]).unwrap();
        let second = apply_writes(&schema, &path, &[req("count", Value::Integer(2))]).unwrap();

        assert_eq!(
            second.changed,
            vec!["count".to_string()],
            "the migration must not fire again"
        );
    }

}
