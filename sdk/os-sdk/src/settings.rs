//! PAS-2: reading an app's own settings against the schema it declared.
//!
//! [`Config`] already loads the app's `config.toml` and answers dotted keys, and
//! that stays the read path: per the plan's dconf-derived split, the broker owns
//! WRITES while everyone reads the file directly. What this module adds is the
//! schema.
//!
//! Without it, a declared default lives in two places at once: in the recipe's
//! `[settings]` block, where the system Settings page reads it, and in the app's
//! own `unwrap_or(…)`, where the app reads it. Nothing keeps the two equal, so
//! the page can honestly show one default while the app uses another. Resolving
//! through the schema makes the declared value the single answer to "what is
//! this key when the user has not set it".
//!
//! Writes are deliberately absent. They belong to the settings broker, which
//! validates against the schema and scope and emits the change signal; an app
//! writing its own file behind the broker's back is what that design exists to
//! prevent.

use arlen_forage_recipe::settings::{SettingScope, SettingType, SettingsItem, SettingsSchema};
use toml::Value;

use crate::config::{Config, ConfigError, FromToml};

/// An app's settings: its config file, read through its declared schema.
pub struct Settings {
    config: Config,
    schema: SettingsSchema,
}

impl Settings {
    /// Load `app_id`'s config and pair it with the schema that app declared.
    pub fn load(app_id: &str, schema: SettingsSchema) -> Result<Self, ConfigError> {
        Ok(Self {
            config: Config::load(app_id)?,
            schema,
        })
    }

    /// Pair an already-loaded config with a schema.
    pub fn new(config: Config, schema: SettingsSchema) -> Self {
        Self { config, schema }
    }

    /// Re-read the config file. The schema is unchanged: it ships with the
    /// package, so it only moves when the app is updated.
    pub fn reload(&mut self) -> Result<(), ConfigError> {
        self.config.reload()
    }

    /// The underlying config, for keys the schema does not declare (an app may
    /// hold private state it does not expose in Settings).
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The declared schema.
    pub fn schema(&self) -> &SettingsSchema {
        &self.schema
    }

    /// The user's value for `key`, falling back to the schema's declared
    /// default. Returns `None` only when the key is unset AND undeclared, or
    /// when the stored value does not fit `T`.
    pub fn get<T: FromToml>(&self, key: &str) -> Option<T> {
        if let Some(value) = self.config.get::<T>(key) {
            return Some(value);
        }
        self.declared_default(key).and_then(T::from_toml)
    }

    /// The raw value for `key`, user value first, then the declared default.
    pub fn get_raw(&self, key: &str) -> Option<&Value> {
        self.config.get_raw(key).or_else(|| self.declared_default(key))
    }

    /// Whether the user has set this key themselves, as opposed to inheriting
    /// the declared default. This is the distinction migrations need: only an
    /// explicitly-set value is carried forward under a renamed key.
    pub fn is_user_set(&self, key: &str) -> bool {
        self.config.get_raw(key).is_some()
    }

    /// The item the schema declares for `key`, if any.
    pub fn item(&self, key: &str) -> Option<&SettingsItem> {
        self.schema
            .sections
            .iter()
            .flat_map(|s| s.items.iter())
            .find(|i| i.key == key)
    }

    /// The declared type of `key`.
    pub fn declared_type(&self, key: &str) -> Option<SettingType> {
        self.item(key).map(|i| i.value_type)
    }

    /// The layer `key` may be written to. An undeclared key has no declared
    /// scope; the broker refuses to write those at all.
    pub fn scope(&self, key: &str) -> Option<SettingScope> {
        self.item(key).map(|i| i.scope)
    }

    /// Every key the schema declares, in declaration order.
    pub fn declared_keys(&self) -> Vec<&str> {
        self.schema
            .sections
            .iter()
            .flat_map(|s| s.items.iter().map(|i| i.key.as_str()))
            .collect()
    }

    /// The schema's default for `key`.
    fn declared_default(&self, key: &str) -> Option<&Value> {
        self.item(key).and_then(|i| i.default.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn schema(items: &[(&str, SettingType, Option<Value>)]) -> SettingsSchema {
        use arlen_forage_recipe::settings::SettingsSection;
        SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "General".into(),
                description: None,
                order: None,
                items: items
                    .iter()
                    .map(|(key, ty, default)| {
                        let mut item = SettingsItem::new(*key, *ty, "L");
                        item.default = default.clone();
                        item
                    })
                    .collect(),
            }],
        }
    }

    fn settings_with(config_toml: &str, schema: SettingsSchema) -> Settings {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(config_toml.as_bytes()).unwrap();
        let config = Config::load_path(file.path()).unwrap();
        // Keep the file alive for the duration of the test by leaking the handle;
        // Config has already read it, so only the path mattered.
        std::mem::forget(file);
        Settings::new(config, schema)
    }

    /// The whole point: an unset key answers with the DECLARED default rather
    /// than nothing, so the app does not need its own copy of that default.
    #[test]
    fn an_unset_key_falls_back_to_the_declared_default() {
        let s = settings_with(
            "other = 1\n",
            schema(&[("theme", SettingType::String, Some(Value::String("dark".into())))]),
        );
        assert_eq!(s.get::<String>("theme").as_deref(), Some("dark"));
    }

    /// A user value must win over the declared default, or the setting would be
    /// unchangeable.
    #[test]
    fn a_user_value_wins_over_the_declared_default() {
        let s = settings_with(
            "theme = \"light\"\n",
            schema(&[("theme", SettingType::String, Some(Value::String("dark".into())))]),
        );
        assert_eq!(s.get::<String>("theme").as_deref(), Some("light"));
    }

    /// Migrations may only carry forward what the user actually chose, so the
    /// two cases must stay distinguishable even though `get` returns a value for
    /// both.
    #[test]
    fn a_defaulted_key_is_not_user_set() {
        let s = settings_with(
            "explicit = true\n",
            schema(&[
                ("explicit", SettingType::Bool, Some(Value::Boolean(false))),
                ("implicit", SettingType::Bool, Some(Value::Boolean(true))),
            ]),
        );
        assert!(s.is_user_set("explicit"));
        assert!(!s.is_user_set("implicit"));
        // Both still read as values.
        assert_eq!(s.get::<bool>("explicit"), Some(true));
        assert_eq!(s.get::<bool>("implicit"), Some(true));
    }

    #[test]
    fn an_undeclared_unset_key_has_no_value() {
        let s = settings_with("", schema(&[("known", SettingType::Bool, None)]));
        assert_eq!(s.get::<String>("unknown"), None);
        assert!(s.item("unknown").is_none());
        assert!(s.scope("unknown").is_none());
    }

    /// A declared key with no default is still declared; it simply has no value
    /// until the user sets one.
    #[test]
    fn a_declared_key_without_a_default_reads_as_unset() {
        let s = settings_with("", schema(&[("token", SettingType::SecretRef, None)]));
        assert_eq!(s.get::<String>("token"), None);
        assert_eq!(s.declared_type("token"), Some(SettingType::SecretRef));
    }

    #[test]
    fn the_schema_answers_type_scope_and_key_list() {
        let s = settings_with(
            "",
            schema(&[
                ("a", SettingType::Bool, None),
                ("b.c", SettingType::Int, None),
            ]),
        );
        assert_eq!(s.declared_keys(), vec!["a", "b.c"]);
        assert_eq!(s.declared_type("b.c"), Some(SettingType::Int));
        assert_eq!(s.scope("a"), Some(SettingScope::User));
    }

    /// A dotted key resolves through the nested table, and its default still
    /// applies when the table is absent entirely.
    #[test]
    fn a_dotted_key_resolves_and_defaults() {
        let sc = schema(&[("window.width", SettingType::Int, Some(Value::Integer(800)))]);
        let set = settings_with("[window]\nwidth = 1200\n", sc.clone());
        assert_eq!(set.get::<i64>("window.width"), Some(1200));

        let unset = settings_with("", sc);
        assert_eq!(unset.get::<i64>("window.width"), Some(800));
    }
}
