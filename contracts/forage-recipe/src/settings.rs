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
    /// Not a value at all: a row that opens the app's own settings window.
    /// The third PAS-6 tier, for the settings a schema genuinely cannot
    /// describe. There is no embedding - Wayland has no XEmbed - so the app
    /// gets its own window.
    Handoff,
    /// A value this vocabulary cannot express, edited as raw TOML.
    ///
    /// The escape hatch, and the reason the type set above can stay closed: an
    /// app with one genuinely irregular value marks that ONE key raw rather than
    /// shipping its own widget. VS Code ("Edit in settings.json") and Chrome
    /// policy (`dict` types as a raw JSON box) converged on the same answer.
    ///
    /// It escapes the type vocabulary, not the model. The key is still declared,
    /// still scope-checked, and the edit still lands at that key alone - so a raw
    /// item cannot reach a key the schema never mentioned.
    Raw,
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

/// Where an enum's choices come from when the package cannot know them.
///
/// The valid set for "which audio output", "which theme", "which browser" is a
/// property of the user's machine, not of the app: it does not exist when the
/// recipe is written and it changes while Settings is open. VS Code has no way
/// to express this, so every extension that needs it ships a free-text field and
/// validates by hand.
///
/// **This is a CLOSED enum, deliberately.** The obvious shape - a command or a
/// path the app names, resolved at render time - would let any third-party
/// package turn its settings page into arbitrary execution or an arbitrary file
/// read, running as Settings. A closed set means the system knows every source it
/// will ever resolve, and a package can only ask for one that already exists.
/// Adding a source is a deliberate change here, which is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSource {
    /// Audio sinks currently present.
    AudioOutputs,
    /// Audio sources currently present.
    AudioInputs,
    /// Themes installed on this machine.
    InstalledThemes,
    /// Locales available on this machine.
    Locales,
    /// Installed applications that handle `http`/`https`.
    Browsers,
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

impl SettingsItem {
    /// A settings item with only what every item must have.
    ///
    /// **Use this rather than a struct literal.** Three downstream crates broke
    /// in two days because a field was added here and every exhaustive literal
    /// in the tree stopped compiling - the schema is a shared contract that
    /// grows, and growing it should not be a breaking change for everyone who
    /// ever constructed one. Set the optional fields on the returned value.
    pub fn new(key: impl Into<String>, value_type: SettingType, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value_type,
            label: label.into(),
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
            handoff: None,
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
}

/// Which of the app's own windows a handoff row opens.
///
/// A NAME the app declares, never a command. That is the whole security design,
/// and it is Android's lesson taken structurally: there, exposing a settings
/// activity to the system settings app exported it to EVERY app, because the
/// mechanism was a public intent filter. Here the app exposes nothing - the
/// system launches the app's ordinary attested entry point and passes the name,
/// so there is no second door to leave open. A free exec string would
/// reintroduce exactly that door, so this type cannot express one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffTarget {
    /// The window name, passed to the app as `--settings-window <name>`.
    /// Constrained to `[a-z0-9-]` because it becomes an argv element.
    pub window: String,
}

/// Whether `app_id` may ask the system to open an app's handoff window.
///
/// Only Settings. A handoff opens a window belonging to another app, on that
/// app's behalf, which is authority no ordinary caller should hold - and
/// Settings is the surface the row is rendered in, so nothing else has a reason
/// to ask. `dev.`-prefixed ids pass in debug builds, the same convention the
/// audit and revoke admissions use, so a cargo-run Settings works without
/// widening the release rule.
pub fn handoff_caller_admitted(app_id: &str) -> bool {
    // EXACT: a handoff opens a window on another app's behalf, so a prefix match
    // would hand that authority to every locally-built binary.
    app_id == "settings" || (cfg!(debug_assertions) && app_id == "dev.arlen-settings")
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
    /// Resolve this enum's choices from live system state instead of declaring
    /// them. Mutually exclusive with `options`.
    #[serde(default)]
    pub options_from: Option<ValueSource>,
    /// Sort position within the section.
    #[serde(default)]
    pub order: Option<i32>,
    /// Extra search terms for the system settings index.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Which layer may hold this key.
    #[serde(default)]
    pub scope: SettingScope,
    /// PAS-6 tier three: the app's own window to open instead of rendering a
    /// control. Required for [`SettingType::Handoff`] and meaningless for
    /// anything else.
    #[serde(default)]
    pub handoff: Option<HandoffTarget>,
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

/// Fatal checks over a declared schema: things that make it unrenderable,
/// ambiguous, or self-contradictory. Called from the crate's [`crate::validate`].
///
/// The split follows the crate's existing convention. A rule lands here when the
/// schema cannot be rendered or applied as written (an enum with nothing to
/// choose from, two items claiming one key, a default that is not among the
/// options). A rule that only signals a probable mistake, while the item still
/// renders correctly, is a lint instead - see [`lint_settings`].
pub(crate) fn validate_settings(
    schema: &SettingsSchema,
    errors: &mut Vec<crate::ValidationError>,
) {
    let mut seen_keys: Vec<&str> = Vec::new();

    for (si, section) in schema.sections.iter().enumerate() {
        if section.label.trim().is_empty() {
            errors.push(crate::err(
                &format!("settings.sections[{si}].label"),
                "must not be empty (it is the rendered heading)",
            ));
        }

        for (ii, item) in section.items.iter().enumerate() {
            let at = |f: &str| format!("settings.sections[{si}].items[{ii}].{f}");

            if !is_valid_setting_key(&item.key) {
                errors.push(crate::err(
                    &at("key"),
                    "must be a dotted path of [a-zA-Z0-9_-] segments",
                ));
            } else if seen_keys.contains(&item.key.as_str()) {
                // Two items claiming one key leaves no answer to "which item
                // owns this value", so it is fatal rather than a hint.
                errors.push(crate::err(
                    &at("key"),
                    "is declared more than once in this schema",
                ));
            } else {
                seen_keys.push(&item.key);
            }

            if item.label.trim().is_empty() {
                errors.push(crate::err(&at("label"), "must not be empty"));
            }

            validate_item_handoff(item, &at, errors);
            validate_item_options(item, &at, errors);
            validate_item_bounds(item, &at, errors);
            validate_item_lifecycle(item, &at, errors);
        }
    }

    validate_visible_when(schema, errors);
}

/// A handoff row opens a window and holds no value, so it must declare a target
/// and must not pretend to have one.
fn validate_item_handoff(
    item: &SettingsItem,
    at: &dyn Fn(&str) -> String,
    errors: &mut Vec<crate::ValidationError>,
) {
    let is_handoff = item.value_type == SettingType::Handoff;
    match (&item.handoff, is_handoff) {
        (None, true) => errors.push(crate::err(
            &at("handoff"),
            "a handoff item must declare which window it opens",
        )),
        (Some(_), false) => errors.push(crate::err(
            &at("handoff"),
            "only a handoff item may declare a window",
        )),
        (Some(target), true) => {
            // It becomes an argv element, so the charset is the guard rather
            // than a tidiness rule.
            // A leading dash is the one the charset alone lets through, and it
            // is the dangerous one: the name is passed as an argv element, so
            // `--flag` would reach the app's argument parser as a FLAG rather
            // than as the value of `--settings-window`.
            if target.window.is_empty()
                || target.window.starts_with('-')
                || !target
                    .window
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                errors.push(crate::err(
                    &at("handoff.window"),
                    "must be a non-empty [a-z0-9-] name",
                ));
            }
        }
        (None, false) => {}
    }
    // A handoff carries no value, so a default or an option list describes
    // something that does not exist and would render a control beside a row
    // whose whole point is that it has none.
    if is_handoff {
        if item.default.is_some() {
            errors.push(crate::err(&at("default"), "a handoff item holds no value"));
        }
        if !item.options.is_empty() || item.options_from.is_some() {
            errors.push(crate::err(&at("options"), "a handoff item holds no value"));
        }
    }
}

/// Enum items must offer a choice, the choices must be distinct, and a declared
/// default must be one of them.
fn validate_item_options(
    item: &SettingsItem,
    at: &impl Fn(&str) -> String,
    errors: &mut Vec<crate::ValidationError>,
) {
    if item.value_type != SettingType::Enum {
        return;
    }

    // Exactly one source of choices. Declaring both leaves the renderer to pick
    // which list is real, and whichever it picked would be right half the time.
    if item.options_from.is_some() {
        if !item.options.is_empty() {
            errors.push(crate::err(
                &at("options_from"),
                "an enum declares either 'options' or 'options_from', not both",
            ));
        }
        // The rest of this function checks the declared list. There isn't one:
        // the choices are whatever the machine has at render time, so neither
        // the duplicate check nor the default-is-one-of-them check can run here.
        // The renderer is what has to cope with a stored value the machine no
        // longer offers.
        return;
    }

    if item.options.is_empty() {
        errors.push(crate::err(
            &at("options"),
            "an enum must declare at least one option",
        ));
        return;
    }

    let mut seen: Vec<&str> = Vec::new();
    for option in &item.options {
        if seen.contains(&option.value.as_str()) {
            errors.push(crate::err(
                &at("options"),
                &format!("option value '{}' is declared more than once", option.value),
            ));
        } else {
            seen.push(&option.value);
        }
    }

    // A default outside the options would ship a value the user can never
    // reselect once changed.
    if let Some(default) = item.default.as_ref().and_then(|v| v.as_str()) {
        if !seen.contains(&default) {
            errors.push(crate::err(
                &at("default"),
                &format!("default '{default}' is not one of the declared options"),
            ));
        }
    }
}

/// An inverted range admits no value at all.
fn validate_item_bounds(
    item: &SettingsItem,
    at: &impl Fn(&str) -> String,
    errors: &mut Vec<crate::ValidationError>,
) {
    if let (Some(min), Some(max)) = (item.min, item.max) {
        if min > max {
            errors.push(crate::err(
                &at("min"),
                "min is greater than max, so no value is valid",
            ));
        }
    }
}

/// Migration metadata must describe a possible history.
fn validate_item_lifecycle(
    item: &SettingsItem,
    at: &impl Fn(&str) -> String,
    errors: &mut Vec<crate::ValidationError>,
) {
    if let (Some(since), Some(removed)) = (item.since, item.removed_in) {
        if removed <= since {
            errors.push(crate::err(
                &at("removed_in"),
                "a key cannot be removed in the version it appeared in, or earlier",
            ));
        }
    }
    if item.renamed_from.iter().any(|old| old == &item.key) {
        errors.push(crate::err(
            &at("renamed_from"),
            "a key cannot be renamed from itself",
        ));
    }
}

/// A visibility condition must name a key this schema actually declares, and
/// must state exactly one condition.
fn validate_visible_when(schema: &SettingsSchema, errors: &mut Vec<crate::ValidationError>) {
    let declared: Vec<&str> = schema
        .sections
        .iter()
        .flat_map(|s| s.items.iter().map(|i| i.key.as_str()))
        .collect();

    for (si, section) in schema.sections.iter().enumerate() {
        for (ii, item) in section.items.iter().enumerate() {
            let Some(cond) = &item.visible_when else { continue };
            let field = format!("settings.sections[{si}].items[{ii}].visible_when");

            // The target may live in any section of the SAME app; there is no
            // way to name another app's key, by construction of the type.
            if !declared.contains(&cond.key.as_str()) {
                errors.push(crate::err(
                    &field,
                    &format!("refers to '{}', which this schema does not declare", cond.key),
                ));
            }
            match (&cond.equals, &cond.in_) {
                (None, None) => errors.push(crate::err(
                    &field,
                    "must state either equals or in",
                )),
                (Some(_), Some(_)) => errors.push(crate::err(
                    &field,
                    "states both equals and in; exactly one applies",
                )),
                _ => {}
            }
        }
    }
}

/// Non-fatal recommendations: the schema renders, but something was probably
/// meant differently.
pub(crate) fn lint_settings(
    schema: &SettingsSchema,
    warnings: &mut Vec<crate::ValidationWarning>,
) {
    for (si, section) in schema.sections.iter().enumerate() {
        if section.items.is_empty() {
            warnings.push(crate::warn(
                &format!("settings.sections[{si}]"),
                "declares no items, so it renders as an empty heading",
            ));
        }
        for (ii, item) in section.items.iter().enumerate() {
            let at = |f: &str| format!("settings.sections[{si}].items[{ii}].{f}");

            // These render fine; the declared extra is simply ignored, which is
            // worth saying out loud rather than silently dropping.
            if !item.value_type.is_numeric()
                && (item.min.is_some() || item.max.is_some() || item.unit.is_some())
            {
                warnings.push(crate::warn(
                    &at("min"),
                    "min, max and unit only apply to numeric types and are ignored here",
                ));
            }
            if item.value_type != SettingType::Enum && !item.options.is_empty() {
                warnings.push(crate::warn(
                    &at("options"),
                    "options only apply to an enum and are ignored here",
                ));
            }
            if item.value_type != SettingType::Enum && item.options_from.is_some() {
                warnings.push(crate::warn(
                    &at("options_from"),
                    "options_from only applies to an enum and is ignored here",
                ));
            }
            // A shipped default names a value the packager guessed; the whole
            // reason for a dynamic source is that the valid values belong to the
            // user's machine. On most machines that guess is simply absent, and
            // the user sees a setting whose stored value is not among its
            // choices.
            if item.options_from.is_some() && item.default.is_some() {
                warnings.push(crate::warn(
                    &at("default"),
                    "a default cannot be relied on with options_from: the machine decides the valid values",
                ));
            }
        }
    }
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

    /// Build a one-section schema from items, for the rule tests.
    fn schema_of(items: Vec<SettingsItem>) -> SettingsSchema {
        SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items,
            }],
        }
    }

    fn item(key: &str, value_type: SettingType) -> SettingsItem {
        SettingsItem::new(key, value_type, "L")
    }

    fn errors_of(schema: &SettingsSchema) -> Vec<String> {
        let mut e = Vec::new();
        validate_settings(schema, &mut e);
        e.into_iter().map(|x| format!("{}: {}", x.field, x.message)).collect()
    }

    fn warnings_of(schema: &SettingsSchema) -> Vec<String> {
        let mut w = Vec::new();
        lint_settings(schema, &mut w);
        w.into_iter().map(|x| format!("{}: {}", x.field, x.message)).collect()
    }

    /// The point of PAS-7: an enum whose valid values live on the user's machine
    /// declares a source instead of a list, and that satisfies the
    /// must-offer-a-choice rule that a bare empty `options` would fail.
    #[test]
    fn an_enum_may_take_its_choices_from_a_system_source() {
        let mut e = item("output", SettingType::Enum);
        e.options_from = Some(ValueSource::AudioOutputs);
        assert!(errors_of(&schema_of(vec![e])).is_empty());
    }

    /// Two lists means the renderer picks which one is real.
    #[test]
    fn declaring_both_options_and_a_source_is_refused() {
        let mut e = item("output", SettingType::Enum);
        e.options_from = Some(ValueSource::AudioOutputs);
        e.options = vec![SettingOption {
            value: "hdmi".into(),
            label: "HDMI".into(),
            description: "The screen".into(),
        }];
        let errs = errors_of(&schema_of(vec![e]));
        assert!(
            errs.iter().any(|e| e.contains("not both")),
            "{errs:?}"
        );
    }

    /// A dynamic source cannot be checked against a declared default, so the
    /// default-is-one-of-the-options rule must not fire on values it can never
    /// see. It stays advisory instead.
    #[test]
    fn a_default_with_a_system_source_is_advised_not_refused() {
        let mut e = item("output", SettingType::Enum);
        e.options_from = Some(ValueSource::AudioOutputs);
        e.default = Some(toml::Value::String("built-in".into()));

        assert!(
            errors_of(&schema_of(vec![e.clone()])).is_empty(),
            "the packager's default must not be a hard error"
        );
        let warns = warnings_of(&schema_of(vec![e]));
        assert!(
            warns.iter().any(|w| w.contains("options_from")),
            "{warns:?}"
        );
    }

    #[test]
    fn a_source_on_a_non_enum_is_flagged_as_ignored() {
        let mut s = item("name", SettingType::String);
        s.options_from = Some(ValueSource::Locales);
        let warns = warnings_of(&schema_of(vec![s]));
        assert!(
            warns.iter().any(|w| w.contains("options_from only applies")),
            "{warns:?}"
        );
    }

    /// A package declares a source by name from a closed set, so it cannot ask
    /// the system to run or read something of its choosing.
    #[test]
    fn a_source_is_named_from_the_closed_set() {
        let parsed: SettingsItem = toml::from_str(
            "key = \"output\"\ntype = \"enum\"\nlabel = \"Output\"\noptions_from = \"audio_outputs\"\n",
        )
        .expect("a known source parses");
        assert_eq!(parsed.options_from, Some(ValueSource::AudioOutputs));

        assert!(
            toml::from_str::<SettingsItem>(
                "key = \"x\"\ntype = \"enum\"\nlabel = \"X\"\noptions_from = \"sh -c 'cat /etc/shadow'\"\n",
            )
            .is_err(),
            "an arbitrary source must not parse"
        );
    }

    #[test]
    fn a_well_formed_schema_has_no_errors() {
        let mut e = item("theme", SettingType::Enum);
        e.options = vec![SettingOption {
            value: "dark".into(),
            label: "Dark".into(),
            description: "d".into(),
        }];
        e.default = Some(toml::Value::String("dark".into()));
        assert!(errors_of(&schema_of(vec![e, item("other", SettingType::Bool)])).is_empty());
    }

    #[test]
    fn an_enum_without_options_is_fatal() {
        let errs = errors_of(&schema_of(vec![item("mode", SettingType::Enum)]));
        assert!(errs.iter().any(|e| e.contains("at least one option")), "{errs:?}");
    }

    /// The shipped default must be selectable, or the user can never get back to
    /// it once they change the value.
    #[test]
    fn an_enum_default_outside_the_options_is_fatal() {
        let mut e = item("mode", SettingType::Enum);
        e.options = vec![SettingOption {
            value: "a".into(),
            label: "A".into(),
            description: "d".into(),
        }];
        e.default = Some(toml::Value::String("b".into()));
        let errs = errors_of(&schema_of(vec![e]));
        assert!(errs.iter().any(|x| x.contains("not one of the declared options")), "{errs:?}");
    }

    #[test]
    fn duplicate_option_values_are_fatal() {
        let mut e = item("mode", SettingType::Enum);
        let opt = |v: &str| SettingOption {
            value: v.into(),
            label: "L".into(),
            description: "d".into(),
        };
        e.options = vec![opt("a"), opt("a")];
        let errs = errors_of(&schema_of(vec![e]));
        assert!(errs.iter().any(|x| x.contains("more than once")), "{errs:?}");
    }

    #[test]
    fn a_duplicate_key_is_fatal() {
        let errs = errors_of(&schema_of(vec![
            item("dup", SettingType::Bool),
            item("dup", SettingType::String),
        ]));
        assert!(errs.iter().any(|e| e.contains("declared more than once")), "{errs:?}");
    }

    /// The duplicate check must span sections, not just look within one.
    #[test]
    fn a_duplicate_key_across_sections_is_fatal() {
        let schema = SettingsSchema {
            version: 1,
            sections: vec![
                SettingsSection {
                    label: "A".into(),
                    description: None,
                    order: None,
                    items: vec![item("shared", SettingType::Bool)],
                },
                SettingsSection {
                    label: "B".into(),
                    description: None,
                    order: None,
                    items: vec![item("shared", SettingType::Bool)],
                },
            ],
        };
        let errs = errors_of(&schema);
        assert!(errs.iter().any(|e| e.contains("declared more than once")), "{errs:?}");
    }

    #[test]
    fn an_inverted_range_is_fatal() {
        let mut i = item("size", SettingType::Int);
        i.min = Some(10.0);
        i.max = Some(1.0);
        let errs = errors_of(&schema_of(vec![i]));
        assert!(errs.iter().any(|e| e.contains("no value is valid")), "{errs:?}");
    }

    #[test]
    fn a_dangling_visible_when_is_fatal() {
        let mut i = item("advanced", SettingType::Bool);
        i.visible_when = Some(VisibleWhen {
            key: "nonexistent".into(),
            equals: Some("true".into()),
            in_: None,
        });
        let errs = errors_of(&schema_of(vec![i]));
        assert!(errs.iter().any(|e| e.contains("does not declare")), "{errs:?}");
    }

    /// The target may live in ANOTHER section of the same app, which must be
    /// accepted - the condition is app-scoped, not section-scoped.
    #[test]
    fn a_visible_when_may_target_another_section() {
        let mut dependent = item("b", SettingType::Bool);
        dependent.visible_when = Some(VisibleWhen {
            key: "a".into(),
            equals: Some("true".into()),
            in_: None,
        });
        let schema = SettingsSchema {
            version: 1,
            sections: vec![
                SettingsSection {
                    label: "One".into(),
                    description: None,
                    order: None,
                    items: vec![item("a", SettingType::Bool)],
                },
                SettingsSection {
                    label: "Two".into(),
                    description: None,
                    order: None,
                    items: vec![dependent],
                },
            ],
        };
        assert!(errors_of(&schema).is_empty(), "{:?}", errors_of(&schema));
    }

    #[test]
    fn a_visible_when_needs_exactly_one_condition() {
        let mut none = item("b", SettingType::Bool);
        none.visible_when = Some(VisibleWhen {
            key: "b".into(),
            equals: None,
            in_: None,
        });
        assert!(errors_of(&schema_of(vec![none]))
            .iter()
            .any(|e| e.contains("either equals or in")));

        let mut both = item("c", SettingType::Bool);
        both.visible_when = Some(VisibleWhen {
            key: "c".into(),
            equals: Some("x".into()),
            in_: Some(vec!["y".into()]),
        });
        assert!(errors_of(&schema_of(vec![both]))
            .iter()
            .any(|e| e.contains("exactly one applies")));
    }

    #[test]
    fn impossible_lifecycle_metadata_is_fatal() {
        let mut removed_too_early = item("old", SettingType::Bool);
        removed_too_early.since = Some(3);
        removed_too_early.removed_in = Some(3);
        assert!(errors_of(&schema_of(vec![removed_too_early]))
            .iter()
            .any(|e| e.contains("cannot be removed")));

        let mut self_rename = item("k", SettingType::Bool);
        self_rename.renamed_from = vec!["k".into()];
        assert!(errors_of(&schema_of(vec![self_rename]))
            .iter()
            .any(|e| e.contains("renamed from itself")));
    }

    #[test]
    fn a_malformed_key_or_empty_label_is_fatal() {
        assert!(errors_of(&schema_of(vec![item("bad key", SettingType::Bool)]))
            .iter()
            .any(|e| e.contains("dotted path")));

        let mut blank = item("k", SettingType::Bool);
        blank.label = "  ".into();
        assert!(errors_of(&schema_of(vec![blank]))
            .iter()
            .any(|e| e.contains("label")));
    }

    /// These render correctly, so they are advice, not failures - the split the
    /// crate draws between validate and lint.
    #[test]
    fn misapplied_extras_are_warnings_not_errors() {
        let mut bounded_string = item("name", SettingType::String);
        bounded_string.min = Some(1.0);
        bounded_string.unit = Some("px".into());
        let mut bool_with_options = item("flag", SettingType::Bool);
        bool_with_options.options = vec![SettingOption {
            value: "a".into(),
            label: "A".into(),
            description: "d".into(),
        }];
        let schema = schema_of(vec![bounded_string, bool_with_options]);

        assert!(errors_of(&schema).is_empty(), "{:?}", errors_of(&schema));
        let warns = warnings_of(&schema);
        assert!(warns.iter().any(|w| w.contains("only apply to numeric types")), "{warns:?}");
        assert!(warns.iter().any(|w| w.contains("only apply to an enum")), "{warns:?}");
    }

    #[test]
    fn an_empty_section_is_a_warning() {
        let schema = schema_of(vec![]);
        assert!(errors_of(&schema).is_empty());
        assert!(warnings_of(&schema).iter().any(|w| w.contains("no items")));
    }


    fn handoff_item(window: &str) -> SettingsItem {
        let mut it = item("advanced", SettingType::Handoff);
        it.handoff = Some(HandoffTarget {
            window: window.into(),
        });
        it
    }

    fn errors_for(items: Vec<SettingsItem>) -> Vec<crate::ValidationError> {
        let schema = SettingsSchema {
            version: 1,
            sections: vec![SettingsSection {
                label: "S".into(),
                description: None,
                order: None,
                items,
            }],
        };
        let mut errors = Vec::new();
        validate_settings(&schema, &mut errors);
        errors
    }

    #[test]
    fn a_handoff_item_declaring_its_window_is_valid() {
        assert!(errors_for(vec![handoff_item("preferences")]).is_empty());
    }

    /// A row that opens nothing is a dead row, so the target is required.
    #[test]
    fn a_handoff_without_a_window_is_refused() {
        let it = item("advanced", SettingType::Handoff);
        assert!(!errors_for(vec![it]).is_empty());
    }

    /// The window name becomes an argv element, so the charset is a guard
    /// rather than a style rule.
    #[test]
    fn a_window_name_that_is_not_a_plain_identifier_is_refused() {
        for bad in ["", "Preferences", "pref erences", "--flag", "a;b", "../x"] {
            assert!(
                !errors_for(vec![handoff_item(bad)]).is_empty(),
                "{bad:?} was accepted as a window name"
            );
        }
    }

    /// A handoff holds no value, so a default or an option list describes
    /// something that does not exist.
    #[test]
    fn a_handoff_cannot_also_claim_a_value() {
        let mut with_default = handoff_item("preferences");
        with_default.default = Some(toml::Value::Boolean(true));
        assert!(!errors_for(vec![with_default]).is_empty());

        let mut with_options = handoff_item("preferences");
        with_options.options = vec![SettingOption {
            value: "a".into(),
            label: "A".into(),
            description: String::new(),
        }];
        assert!(!errors_for(vec![with_options]).is_empty());
    }

    /// Only a handoff row may name a window; anything else declaring one is a
    /// schema that will not render the way its author expects.
    #[test]
    fn only_a_handoff_may_declare_a_window() {
        let mut it = item("colour", SettingType::Bool);
        it.handoff = Some(HandoffTarget {
            window: "preferences".into(),
        });
        assert!(!errors_for(vec![it]).is_empty());
    }

    /// Opening another app's window on its behalf is authority only Settings
    /// holds. This is the check that keeps the handoff from becoming the
    /// exported activity Android shipped.
    #[test]
    fn only_settings_may_trigger_a_handoff() {
        assert!(handoff_caller_admitted("settings"));
        for other in ["org.example.App", "modulesd", "ai-agent", "", "settings.evil"] {
            assert!(
                !handoff_caller_admitted(other),
                "{other:?} was admitted to open another app's window"
            );
        }
    }
}
