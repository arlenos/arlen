//! PAS-4: resolving an app-id to its declared schema and config file.
//!
//! The schema is a PACKAGE artifact: `forage` ships it in the recipe alongside
//! `[capabilities]`, and installing the app lands it in a known directory. That
//! is what lets Settings render an app's page even if the app has never run,
//! which a runtime-only `shell.settings.register` could not.
//!
//! **The lookup re-reads on every call rather than caching at boot.** The plan
//! is explicit that the registry is live - install, update and uninstall are a
//! delta, not a restart - and this repo has already been bitten by the other
//! shape: the online-accounts daemon had to be changed to re-read its configs
//! per call because a cached copy went stale the moment anything changed on
//! disk. Re-reading a small TOML file per write is far cheaper than serving a
//! schema that no longer matches the installed app.
//!
//! **`app_id` arrives over the socket and is joined into a path**, so it is
//! validated before it ever touches the filesystem. Without that, an id
//! containing `..` or `/` would let a caller point the broker at a file outside
//! the schema directory entirely.

use std::path::{Path, PathBuf};

use arlen_forage_recipe::settings::SettingsSchema;

use crate::serve::{AppRegistry, AppSettings};

/// Whether an app-id is safe to use as a path component.
///
/// Reverse-DNS ids are alphanumerics, dots, dashes and underscores. Anything
/// else - a separator, `..`, an empty string - is refused rather than sanitised,
/// because a "cleaned" id would still address a file the caller chose.
pub fn is_safe_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 128
        && app_id != "."
        && app_id != ".."
        && app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// A registry backed by directories of installed schemas.
///
/// `schema_dirs` are searched in order, so a user-installed app can shadow a
/// system one the same way the rest of Arlen's layering works.
pub struct DirectoryRegistry {
    schema_dirs: Vec<PathBuf>,
    config_dir: PathBuf,
}

impl DirectoryRegistry {
    /// Build a registry over explicit directories.
    pub fn new(schema_dirs: Vec<PathBuf>, config_dir: PathBuf) -> Self {
        Self {
            schema_dirs,
            config_dir,
        }
    }

    /// Where an app's schema file lives within a schema directory.
    fn schema_path(dir: &Path, app_id: &str) -> PathBuf {
        dir.join(format!("{app_id}.toml"))
    }

    /// The app's own config file.
    fn config_path(&self, app_id: &str) -> PathBuf {
        self.config_dir.join(format!("{app_id}.toml"))
    }

    /// Read and parse the first schema found for `app_id`.
    fn read_schema(&self, app_id: &str) -> Option<SettingsSchema> {
        for dir in &self.schema_dirs {
            let path = Self::schema_path(dir, app_id);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match toml::from_str::<SettingsSchema>(&text) {
                Ok(schema) => return Some(schema),
                Err(e) => {
                    // A malformed schema is skipped rather than falling through
                    // to a laxer one: serving a shadowed schema would validate
                    // writes against rules the installed app never declared.
                    eprintln!("settings-broker: {} is not a valid schema: {e}", path.display());
                    return None;
                }
            }
        }
        None
    }
}

impl AppRegistry for DirectoryRegistry {
    fn lookup(&self, app_id: &str) -> Option<AppSettings> {
        if !is_safe_app_id(app_id) {
            return None;
        }
        let schema = self.read_schema(app_id)?;
        Some(AppSettings {
            schema,
            config_path: self.config_path(app_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"
version = 1
[[sections]]
label = "General"
[[sections.items]]
key = "theme"
type = "string"
label = "Theme"
"#;

    fn registry_with(schema_files: &[(&str, &str)]) -> (tempfile::TempDir, DirectoryRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        let configs = dir.path().join("configs");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::create_dir_all(&configs).unwrap();
        for (name, body) in schema_files {
            std::fs::write(schemas.join(name), body).unwrap();
        }
        (
            dir,
            DirectoryRegistry::new(vec![schemas], configs),
        )
    }

    #[test]
    fn an_installed_schema_resolves() {
        let (_d, reg) = registry_with(&[("org.example.App.toml", SCHEMA)]);
        let found = reg.lookup("org.example.App").expect("should resolve");
        assert_eq!(found.schema.version, 1);
        assert_eq!(found.schema.sections[0].items[0].key, "theme");
        assert!(found.config_path.ends_with("org.example.App.toml"));
    }

    #[test]
    fn an_app_with_no_schema_does_not_resolve() {
        let (_d, reg) = registry_with(&[("org.example.App.toml", SCHEMA)]);
        assert!(reg.lookup("org.other.App").is_none());
    }

    /// The id is joined into a path, so a traversing id must never reach the
    /// filesystem - not even to be "cleaned" first.
    #[test]
    fn a_traversing_app_id_is_refused() {
        let (_d, reg) = registry_with(&[("org.example.App.toml", SCHEMA)]);
        for hostile in [
            "../../etc/passwd",
            "..",
            ".",
            "org/example",
            "org.example.App/../../x",
            "",
            "a b",
        ] {
            assert!(reg.lookup(hostile).is_none(), "{hostile} should be refused");
            assert!(!is_safe_app_id(hostile), "{hostile} should be unsafe");
        }
    }

    #[test]
    fn ordinary_reverse_dns_ids_are_accepted() {
        for ok in ["org.example.App", "a", "app-name_2", "org.example.App.v2"] {
            assert!(is_safe_app_id(ok), "{ok} should be safe");
        }
        assert!(!is_safe_app_id(&"a".repeat(129)));
    }

    /// The first directory wins, so a user-installed app shadows a system one.
    #[test]
    fn the_first_schema_directory_wins() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user");
        let system = dir.path().join("system");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(user.join("org.example.App.toml"), SCHEMA).unwrap();
        std::fs::write(
            system.join("org.example.App.toml"),
            SCHEMA.replace("version = 1", "version = 9"),
        )
        .unwrap();

        let reg = DirectoryRegistry::new(vec![user, system], dir.path().join("configs"));
        assert_eq!(reg.lookup("org.example.App").unwrap().schema.version, 1);
    }

    /// A malformed schema must not fall through to a lower-precedence one: that
    /// would validate writes against rules the installed app never declared.
    #[test]
    fn a_malformed_schema_does_not_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user");
        let system = dir.path().join("system");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(user.join("org.example.App.toml"), "not = = toml").unwrap();
        std::fs::write(system.join("org.example.App.toml"), SCHEMA).unwrap();

        let reg = DirectoryRegistry::new(vec![user, system], dir.path().join("configs"));
        assert!(reg.lookup("org.example.App").is_none());
    }

    /// The registry is LIVE: a schema installed after the registry was built is
    /// visible without restarting, and one removed stops resolving.
    #[test]
    fn the_registry_reflects_install_and_uninstall_without_a_restart() {
        let (dir, reg) = registry_with(&[]);
        let schemas = dir.path().join("schemas");

        assert!(reg.lookup("org.example.App").is_none(), "not installed yet");

        std::fs::write(schemas.join("org.example.App.toml"), SCHEMA).unwrap();
        assert!(reg.lookup("org.example.App").is_some(), "install not seen");

        std::fs::remove_file(schemas.join("org.example.App.toml")).unwrap();
        assert!(reg.lookup("org.example.App").is_none(), "uninstall not seen");
    }

    /// An updated schema is served without a restart, which is the case a
    /// parse-once-at-boot registry gets wrong.
    #[test]
    fn an_updated_schema_is_served_immediately() {
        let (dir, reg) = registry_with(&[("org.example.App.toml", SCHEMA)]);
        assert_eq!(reg.lookup("org.example.App").unwrap().schema.version, 1);

        std::fs::write(
            dir.path().join("schemas").join("org.example.App.toml"),
            SCHEMA.replace("version = 1", "version = 2"),
        )
        .unwrap();
        assert_eq!(reg.lookup("org.example.App").unwrap().schema.version, 2);
    }
}
