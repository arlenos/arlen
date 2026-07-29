//! PAS-1: the settings schema an app declares in its recipe, next to
//! `[capabilities]` (per-app-settings-plan.md section 1).
//!
//! The schema ships with the PACKAGE rather than being registered at runtime.
//! `shell.settings.register` is a runtime call, so a runtime-only model would
//! mean Settings knows an app's settings only while that app is running - you
//! could not configure an app you had never launched. Package-first (GSettings'
//! install-time model) makes the schema available from install; runtime
//! registration is the delta on top.
//!
//! Modelled on VS Code's `contributes.configuration`, whose per-property shape
//! is roughly half presentation and lifecycle metadata on top of the typing, and
//! on KDE's `.kcfg`, which converged independently on the same field set.
//!
//! **One field is deliberately absent.** VS Code lets a property declare a
//! `policy` that a system-wide policy can always override, beating default, user
//! and workspace values alike. On a sovereign OS that is precisely the mechanism
//! to omit: nothing here may lock the user out of their own machine.

use serde::{Deserialize, Serialize};

/// The settings an app declares, as one schema per app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSchema {
    /// Schema version, bumped when keys are added, renamed or removed. Used by
    /// the migration path (`renamed_from` / `since` / `removed_in`).
    pub version: u32,
    /// The sections, rendered in `order` then declaration order.
    #[serde(default)]
    pub sections: Vec<SettingsSection>,
}

/// One group of settings items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSection {
    /// The section heading.
    pub label: String,
    /// An optional sentence under the heading.
    #[serde(default)]
    pub description: Option<String>,
    /// Sort position; sections without one keep declaration order after those
    /// that have one.
    #[serde(default)]
    pub order: Option<i32>,
    /// The items in this section.
    #[serde(default)]
    pub items: Vec<SettingsItem>,
}

/// The value type of one setting. Closed on purpose: the system renders these
/// and only these, and an app needing anything else uses the escape hatch
/// (a raw editor or a handoff to its own window) rather than shipping a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    /// A switch.
    Bool,
    /// A whole number, optionally bounded by `min`/`max` with a `unit`.
    Int,
    /// A decimal number, same bounding.
    Float,
    /// A single-line string.
    String,
    /// One of `options`.
    Enum,
    /// An ordered list of strings.
    StringList,
    /// A filesystem path (rendered with a picker).
    Path,
    /// A colour.
    Color,
    /// A key combination.
    Keybind,
    /// A time span, with `unit` naming the granularity.
    Duration,
    /// A REFERENCE to a secret held in the vault, never the secret itself: the
    /// schema and the app's `config.toml` only ever carry the handle.
    SecretRef,
}

/// Which layer a key may legally be written to. The EDITOR enforces this, not
/// the caller (VS Code's `configurationEditing` rejects writes aimed at the
/// wrong file), so the broker refuses a write whose target layer does not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingScope {
    /// Writable in the user's own config (the default).
    #[default]
    User,
    /// Machine-wide; not carried between machines by sync.
    Machine,
    /// Shipped defaults only; the user cannot write this key.
    DefaultsOnly,
}

/// One option of an `enum` setting. The description is not optional: an enum
/// whose options carry no explanation cannot be rendered in plain language,
/// which is the whole point of a system-rendered settings page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingOption {
    /// The stored value.
    pub value: String,
    /// The option's short name.
    pub label: String,
    /// What choosing it means.
    pub description: String,
}

/// A conditional-visibility rule over ANOTHER key of the SAME app. Cross-app
/// conditions are not expressible: an app must not be able to make its own page
/// depend on another app's configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibleWhen {
    /// The other key in this app's schema.
    pub key: String,
    /// Show when that key equals this value.
    #[serde(default)]
    pub equals: Option<String>,
    /// Show when that key is one of these values.
    #[serde(default)]
    pub in_: Option<Vec<String>>,
}

/// One declared setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsItem {
    /// Dotted path into the app's own `config.toml`.
    pub key: String,
    /// The value type.
    #[serde(rename = "type")]
    pub value_type: SettingType,
    /// Short label.
    pub label: String,
    /// A plain-language explanation.
    #[serde(default)]
    pub description: Option<String>,
    /// The shipped default, as it appears in the config.
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// Inclusive lower bound, numerics only.
    #[serde(default)]
    pub min: Option<f64>,
    /// Inclusive upper bound, numerics only.
    #[serde(default)]
    pub max: Option<f64>,
    /// What the number counts (`seconds`, `MB`, `px`), so the renderer can say
    /// it instead of showing a bare figure.
    #[serde(default)]
    pub unit: Option<String>,
    /// The choices, for `enum`.
    #[serde(default)]
    pub options: Vec<SettingOption>,
    /// Sort position within the section.
    #[serde(default)]
    pub order: Option<i32>,
    /// Extra search terms for the system settings index.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Which layer may hold this key.
    #[serde(default)]
    pub scope: SettingScope,
    /// Free tags; `experimental` and `advanced` are the rendered ones.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Present in the schema but not shown by default.
    #[serde(default)]
    pub included: Option<bool>,
    /// Why this key is going away.
    #[serde(default)]
    pub deprecated_message: Option<String>,
    /// The key that supersedes this one.
    #[serde(default)]
    pub replaced_by: Option<String>,
    /// Former names of this key. Forwarding is unconditional; the warning only
    /// appears once `since` is old enough.
    #[serde(default)]
    pub renamed_from: Vec<String>,
    /// Schema version this key appeared in.
    #[serde(default)]
    pub since: Option<u32>,
    /// Schema version this key is removed in.
    #[serde(default)]
    pub removed_in: Option<u32>,
    /// Show only when another key of this app has a given value.
    #[serde(default)]
    pub visible_when: Option<VisibleWhen>,
}

impl SettingType {
    /// Whether `min`/`max`/`unit` are meaningful for this type.
    pub fn is_numeric(self) -> bool {
        matches!(self, SettingType::Int | SettingType::Float | SettingType::Duration)
    }
}

/// Whether a key is a well-formed dotted path (`a`, `a.b`, `window.width`).
/// Rejects empty segments so a key cannot address something the config format
/// cannot express.
pub fn is_valid_setting_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotted_key_is_valid_and_an_empty_segment_is_not() {
        assert!(is_valid_setting_key("theme"));
        assert!(is_valid_setting_key("window.width"));
        assert!(is_valid_setting_key("a.b.c_d-e"));
        assert!(!is_valid_setting_key(""));
        assert!(!is_valid_setting_key("."));
        assert!(!is_valid_setting_key("a..b"));
        assert!(!is_valid_setting_key(".leading"));
        assert!(!is_valid_setting_key("trailing."));
        assert!(!is_valid_setting_key("has space"));
        assert!(!is_valid_setting_key("has/slash"));
    }

    #[test]
    fn numeric_types_take_bounds_and_others_do_not() {
        assert!(SettingType::Int.is_numeric());
        assert!(SettingType::Float.is_numeric());
        assert!(SettingType::Duration.is_numeric());
        assert!(!SettingType::Bool.is_numeric());
        assert!(!SettingType::String.is_numeric());
        assert!(!SettingType::Enum.is_numeric());
        assert!(!SettingType::SecretRef.is_numeric());
    }

    #[test]
    fn scope_defaults_to_user() {
        assert_eq!(SettingScope::default(), SettingScope::User);
    }

    /// The whole block round-trips from the TOML an app would actually write,
    /// including the `type` rename and snake_case type names.
    #[test]
    fn a_declared_schema_parses_from_toml() {
        let toml_str = r#"
version = 1

[[sections]]
label = "Appearance"
description = "How it looks"
order = 1

[[sections.items]]
key = "theme"
type = "enum"
label = "Theme"
description = "Which colour scheme to use"
default = "dark"
keywords = ["colour", "dark mode"]
scope = "user"

[[sections.items.options]]
value = "dark"
label = "Dark"
description = "Light text on a dark background"

[[sections.items.options]]
value = "light"
label = "Light"
description = "Dark text on a light background"

[[sections.items]]
key = "window.width"
type = "int"
label = "Window width"
min = 400.0
max = 3840.0
unit = "px"
scope = "machine"
"#;
        let schema: SettingsSchema = toml::from_str(toml_str).unwrap();
        assert_eq!(schema.version, 1);
        assert_eq!(schema.sections.len(), 1);
        let items = &schema.sections[0].items;
        assert_eq!(items.len(), 2);

        assert_eq!(items[0].key, "theme");
        assert_eq!(items[0].value_type, SettingType::Enum);
        assert_eq!(items[0].options.len(), 2);
        assert_eq!(items[0].options[0].description, "Light text on a dark background");
        assert_eq!(items[0].scope, SettingScope::User);

        assert_eq!(items[1].value_type, SettingType::Int);
        assert_eq!(items[1].unit.as_deref(), Some("px"));
        assert_eq!(items[1].scope, SettingScope::Machine);
        // Unset optional metadata stays absent rather than defaulting to
        // something the app did not declare.
        assert!(items[1].description.is_none());
        assert!(items[1].visible_when.is_none());
    }

    #[test]
    fn a_minimal_item_needs_only_key_type_and_label() {
        let schema: SettingsSchema = toml::from_str(
            r#"
version = 2
[[sections]]
label = "General"
[[sections.items]]
key = "enabled"
type = "bool"
label = "Enabled"
"#,
        )
        .unwrap();
        let item = &schema.sections[0].items[0];
        assert_eq!(item.value_type, SettingType::Bool);
        assert_eq!(item.scope, SettingScope::User);
        assert!(item.keywords.is_empty());
        assert!(item.renamed_from.is_empty());
    }

    /// `string_list` and `secret_ref` must arrive as snake_case, matching how an
    /// app writes them.
    #[test]
    fn multiword_types_use_snake_case_on_the_wire() {
        let schema: SettingsSchema = toml::from_str(
            r#"
version = 1
[[sections]]
label = "S"
[[sections.items]]
key = "hosts"
type = "string_list"
label = "Hosts"
[[sections.items]]
key = "token"
type = "secret_ref"
label = "API token"
"#,
        )
        .unwrap();
        assert_eq!(schema.sections[0].items[0].value_type, SettingType::StringList);
        assert_eq!(schema.sections[0].items[1].value_type, SettingType::SecretRef);
    }
    /// The block must parse as part of a REAL recipe, through the crate's own
    /// `parse`, next to `[capabilities]` - that is the contract PAS-1 states.
    #[test]
    fn a_recipe_carries_its_settings_block() {
        let recipe = crate::parse(
            r#"
[recipe]
id = "org.example.Notes"
name = "Notes"
summary = "plain notes"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/notes"
commit = "0000000000000000000000000000000000000000"

[capabilities]
notifications = true

[settings]
version = 1

[[settings.sections]]
label = "General"

[[settings.sections.items]]
key = "autosave"
type = "bool"
label = "Autosave"
"#,
        )
        .unwrap();
        let schema = recipe.settings.expect("the settings block should parse");
        assert_eq!(schema.version, 1);
        assert_eq!(schema.sections[0].items[0].key, "autosave");
        // The neighbouring capabilities block is unaffected.
        assert!(recipe.capabilities.unwrap().notifications);
    }

    /// A recipe without the block stays valid: settings are optional.
    #[test]
    fn a_recipe_without_settings_is_still_valid() {
        let recipe = crate::parse(
            r#"
[recipe]
id = "org.example.Tool"
name = "Tool"
summary = "a tool"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/tool"
commit = "0000000000000000000000000000000000000000"
"#,
        )
        .unwrap();
        assert!(recipe.settings.is_none());
        // Absent settings must not itself be a validation error. Other fields of
        // this deliberately minimal fixture may be, so assert precisely.
        let errors = crate::validate(&recipe);
        assert!(
            !errors.iter().any(|e| e.field.starts_with("settings")),
            "settings must be optional, got {errors:?}"
        );
    }

}
