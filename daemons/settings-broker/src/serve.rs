//! Answering one request.
//!
//! The broker resolves an app-id to that app's declared schema and config path
//! through an [`AppRegistry`]. That lookup is a seam rather than a hardcoded
//! path because PAS-4 makes the registry LIVE: apps register, deregister and
//! delta as they are installed, updated and removed, so a parse-once-at-boot
//! map cannot survive an app update. Keeping it behind a trait means the
//! request path is testable now and the real registry drops in later without
//! touching this logic.
//!
//! An unknown app is refused rather than defaulted. Guessing a config path for
//! an app whose schema we do not have would mean writing an unvalidated key into
//! a file nobody declared.

use std::path::PathBuf;

use arlen_forage_recipe::settings::SettingsSchema;

use crate::broker::{apply_writes, BrokerError};
use crate::decide::WriteRequest;
use crate::protocol::{Request, Response};

/// Where an app's declared schema and config file live.
pub struct AppSettings {
    /// The schema the app shipped with its package.
    pub schema: SettingsSchema,
    /// The app's own `config.toml`.
    pub config_path: PathBuf,
}

/// Resolves an app-id to its settings. Implemented for real by the live
/// registry (PAS-4); implemented in tests by a fixed map.
pub trait AppRegistry {
    /// The app's schema and config path, or `None` when the app is unknown.
    fn lookup(&self, app_id: &str) -> Option<AppSettings>;
}

/// Answer one request against `registry`.
pub fn answer(registry: &dyn AppRegistry, request: Request) -> Response {
    match request {
        Request::Write { app_id, writes } => {
            let Some(app) = registry.lookup(&app_id) else {
                return Response::UnknownApp { app_id };
            };
            let requests: Vec<WriteRequest> = writes
                .into_iter()
                .map(|w| WriteRequest {
                    app_id: app_id.clone(),
                    key: w.key,
                    value: w.value,
                })
                .collect();

            match apply_writes(&app.schema, &app.config_path, &requests) {
                Ok(signal) => signal.into(),
                Err(BrokerError::Refused { key, rejection }) => Response::Refused {
                    key,
                    reason: rejection.to_string(),
                },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::settings::{
        SettingScope, SettingType, SettingsItem, SettingsSection,
    };
    use crate::protocol::KeyWrite;
    use toml::Value;

    struct OneApp {
        app_id: String,
        schema: SettingsSchema,
        path: PathBuf,
    }

    impl AppRegistry for OneApp {
        fn lookup(&self, app_id: &str) -> Option<AppSettings> {
            (app_id == self.app_id).then(|| AppSettings {
                schema: self.schema.clone(),
                config_path: self.path.clone(),
            })
        }
    }

    fn schema() -> SettingsSchema {
        SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items: vec![SettingsItem {
                    key: "theme".into(),
                    value_type: SettingType::String,
                    label: "Theme".into(),
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
                    renamed_from: Vec::new(),
                    since: None,
                    removed_in: None,
                    visible_when: None,
                }],
            }],
        }
    }

    fn registry() -> (tempfile::TempDir, OneApp) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        (
            dir,
            OneApp {
                app_id: "org.example.App".into(),
                schema: schema(),
                path,
            },
        )
    }

    fn write(app_id: &str, key: &str, value: Value) -> Request {
        Request::Write {
            app_id: app_id.into(),
            writes: vec![KeyWrite {
                key: key.into(),
                value,
            }],
        }
    }

    #[test]
    fn a_legal_write_answers_with_the_changed_key() {
        let (_d, reg) = registry();
        let response = answer(
            &reg,
            write("org.example.App", "theme", Value::String("dark".into())),
        );
        assert_eq!(
            response,
            Response::Changed {
                app_id: "org.example.App".into(),
                changed: vec!["theme".into()]
            }
        );
    }

    /// An app with no declared schema is refused, not defaulted: guessing a path
    /// would mean writing an unvalidated key into a file nobody declared.
    #[test]
    fn an_unknown_app_is_refused() {
        let (_d, reg) = registry();
        let response = answer(
            &reg,
            write("org.other.App", "theme", Value::String("dark".into())),
        );
        assert_eq!(
            response,
            Response::UnknownApp {
                app_id: "org.other.App".into()
            }
        );
    }

    #[test]
    fn a_refused_key_is_named_in_the_answer() {
        let (_d, reg) = registry();
        let response = answer(&reg, write("org.example.App", "nope", Value::Boolean(true)));
        match response {
            Response::Refused { key, reason } => {
                assert_eq!(key, "nope");
                assert!(reason.contains("does not declare"), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// Re-writing the same value answers with an empty changed set, so a
    /// subscriber is not woken for nothing.
    #[test]
    fn a_no_op_write_answers_with_no_changed_keys() {
        let (_d, reg) = registry();
        let first = write("org.example.App", "theme", Value::String("dark".into()));
        answer(&reg, first.clone());

        match answer(&reg, first) {
            Response::Changed { changed, .. } => assert!(changed.is_empty(), "{changed:?}"),
            other => panic!("expected Changed, got {other:?}"),
        }
    }
}
