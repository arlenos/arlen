//! The Settings-adapter manifest model + its validation (integration-packages-plan.md IP-R2).
//!
//! An adapter is declarative, code-free data: it names the config files an app
//! keeps (`[sources]`) and the settings to expose over them (`[[settings]]`), and
//! the privileged Settings app does every read/write itself through
//! `arlen-config-format`. This module parses that manifest into a typed model and
//! validates it fail-closed: every source path is confined to the user-config
//! allowlist ([`crate::allowlist`]) and every setting references a declared
//! source. Resolving a source's glob to concrete files and the write engine build
//! on this.

use crate::allowlist::{resolve_under_allowlist, AllowlistError};
use arlen_config_format::Format;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The on-disk format of a source, as the adapter names it (kebab-case in TOML).
/// A thin wire enum that maps to the [`Format`] the format-handler library
/// dispatches on, so the adapter does not depend on that enum's serde shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatName {
    /// TOML.
    Toml,
    /// JSON / JSONC.
    Json,
    /// INI / `.conf`.
    Ini,
    /// Firefox `prefs.js` (`user_pref(...)` lines).
    FirefoxPrefs,
    /// `.env` files.
    Env,
    /// Flat `key = value`.
    Flat,
}

impl FormatName {
    /// The [`Format`] this names, for `arlen_config_format::handler_for`.
    pub fn to_format(self) -> Format {
        match self {
            FormatName::Toml => Format::Toml,
            FormatName::Json => Format::Json,
            FormatName::Ini => Format::Ini,
            FormatName::FirefoxPrefs => Format::FirefoxPrefs,
            FormatName::Env => Format::Env,
            FormatName::Flat => Format::Flat,
        }
    }
}

/// What to do when a source's path glob matches multiple files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStrategy {
    /// Write the most-recently-modified match (multi-profile apps).
    LastUsed,
    /// Write every match.
    All,
    /// Prompt once and remember. The safe default for a genuinely ambiguous glob.
    Ask,
}

impl Default for InstanceStrategy {
    /// `ask` is the recommended default (Decided 4): when the glob is ambiguous,
    /// the safe choice is to let the user pick, not to guess.
    fn default() -> Self {
        InstanceStrategy::Ask
    }
}

/// When an edit may be written back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteStrategy {
    /// Write immediately.
    Anytime,
    /// Disable edits while the app runs and write only when it is closed (some
    /// apps rewrite their config on exit, discarding live edits).
    RequiresAppClosed,
}

/// The value type of an exposed setting (drives the Settings render + coercion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    /// A free string.
    String,
    /// An integer.
    Int,
    /// A boolean.
    Bool,
    /// A floating-point number.
    Float,
    /// One of an enumerated set (the choices are render metadata, not modelled here).
    Enum,
}

/// One config source: a path glob, its format, and the multi-match strategy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpec {
    /// The source path (a `~`-rooted glob, e.g. `~/.mozilla/firefox/*/prefs.js`).
    pub path: String,
    /// The on-disk format.
    pub format: FormatName,
    /// What to do when the glob matches multiple files.
    #[serde(default)]
    pub instance_strategy: InstanceStrategy,
}

/// One exposed setting: a key in a named source, with render + verify metadata.
// No `Eq`: `default` is a `toml::Value`, which carries a float and so is only
// `PartialEq`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingSpec {
    /// The key path within the source file.
    pub key: String,
    /// The name of the [`SourceSpec`] this setting reads/writes.
    pub source: String,
    /// The human label shown in Settings.
    pub label: String,
    /// The value type.
    #[serde(rename = "type")]
    pub ty: SettingType,
    /// The default value, if any (kept loosely typed; coerced by `ty` downstream).
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// The Settings section to group under.
    #[serde(default)]
    pub section: Option<String>,
    /// Read the value back after a write and confirm it (catches an app that
    /// renamed/removed the key in a new version).
    #[serde(default)]
    pub verify: bool,
    /// Shown read-only (with a lock indicator), never editable.
    #[serde(default)]
    pub readonly: bool,
}

/// The `[adapter]` table: the schema version and the write strategy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterMeta {
    /// The adapter schema version (only `"1.0"` is understood today).
    pub schema_version: String,
    /// When edits may be written.
    pub write_strategy: WriteStrategy,
}

/// A full Settings-adapter manifest: the `[adapter]` table, the `[sources]`, and
/// the `[[settings]]`.
// No `Eq`: it carries `SettingSpec`s, whose `toml::Value` default is `PartialEq` only.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterManifest {
    /// The `[adapter]` metadata.
    pub adapter: AdapterMeta,
    /// The named sources.
    #[serde(default)]
    pub sources: BTreeMap<String, SourceSpec>,
    /// The exposed settings.
    #[serde(default)]
    pub settings: Vec<SettingSpec>,
}

/// The schema version this interpreter understands.
pub const SCHEMA_VERSION: &str = "1.0";

/// Why an adapter manifest was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// The TOML did not parse, or carried an unknown field / wrong type.
    #[error("malformed adapter: {0}")]
    Parse(String),
    /// The `schema_version` is not one this interpreter understands.
    #[error("unsupported adapter schema_version {0:?} (this build understands {SCHEMA_VERSION:?})")]
    UnsupportedVersion(String),
    /// A source path failed the user-config allowlist gate.
    #[error("source {name:?}: {error}")]
    Source {
        /// The offending source name.
        name: String,
        /// The allowlist failure.
        error: AllowlistError,
    },
    /// A setting referenced a source name that no `[sources]` entry declares.
    #[error("setting {key:?} references unknown source {source_name:?}")]
    DanglingSource {
        /// The setting key.
        key: String,
        /// The unresolved source name. (Not named `source`: thiserror would treat
        /// a `source` field as the error cause, which a `String` is not.)
        source_name: String,
    },
}

impl AdapterManifest {
    /// Parse and validate an adapter manifest from TOML, confining its source
    /// paths to the user-config allowlist under `home`, fail-closed.
    pub fn parse(toml_text: &str, home: &Path) -> Result<Self, AdapterError> {
        let manifest: AdapterManifest =
            toml::from_str(toml_text).map_err(|e| AdapterError::Parse(e.to_string()))?;
        manifest.validate(home)?;
        Ok(manifest)
    }

    /// Validate the manifest: the schema version is understood, every source path
    /// is inside the user-config allowlist, and every setting references a
    /// declared source.
    pub fn validate(&self, home: &Path) -> Result<(), AdapterError> {
        if self.adapter.schema_version != SCHEMA_VERSION {
            return Err(AdapterError::UnsupportedVersion(
                self.adapter.schema_version.clone(),
            ));
        }
        for (name, source) in &self.sources {
            resolve_under_allowlist(&source.path, home).map_err(|error| AdapterError::Source {
                name: name.clone(),
                error,
            })?;
        }
        for setting in &self.settings {
            if !self.sources.contains_key(&setting.source) {
                return Err(AdapterError::DanglingSource {
                    key: setting.key.clone(),
                    source_name: setting.source.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    const FIREFOX: &str = r#"
        [adapter]
        schema_version = "1.0"
        write_strategy = "requires_app_closed"

        [sources]
        prefs = { path = "~/.mozilla/firefox/*/prefs.js", format = "firefox-prefs", instance_strategy = "last_used" }

        [[settings]]
        key = "browser.startup.homepage"
        source = "prefs"
        label = "Homepage"
        type = "string"
        default = "about:home"
        section = "General"
        verify = true
    "#;

    #[test]
    fn parses_the_firefox_adapter() {
        let m = AdapterManifest::parse(FIREFOX, &home()).unwrap();
        assert_eq!(m.adapter.write_strategy, WriteStrategy::RequiresAppClosed);
        let prefs = &m.sources["prefs"];
        assert_eq!(prefs.format, FormatName::FirefoxPrefs);
        assert_eq!(prefs.format.to_format(), Format::FirefoxPrefs);
        assert_eq!(prefs.instance_strategy, InstanceStrategy::LastUsed);
        assert_eq!(m.settings.len(), 1);
        assert!(m.settings[0].verify);
        assert_eq!(m.settings[0].source, "prefs");
    }

    #[test]
    fn an_unspecified_instance_strategy_defaults_to_ask() {
        let m = AdapterManifest::parse(
            r#"
            [adapter]
            schema_version = "1.0"
            write_strategy = "anytime"
            [sources]
            cfg = { path = "~/.config/app/config.toml", format = "toml" }
            "#,
            &home(),
        )
        .unwrap();
        assert_eq!(m.sources["cfg"].instance_strategy, InstanceStrategy::Ask);
    }

    #[test]
    fn refuses_a_source_outside_the_allowlist() {
        let manifest = r#"
            [adapter]
            schema_version = "1.0"
            write_strategy = "anytime"
            [sources]
            evil = { path = "/etc/passwd", format = "flat" }
        "#;
        match AdapterManifest::parse(manifest, &home()) {
            Err(AdapterError::Source { name, .. }) => assert_eq!(name, "evil"),
            other => panic!("expected an allowlist rejection, got {other:?}"),
        }
    }

    #[test]
    fn refuses_a_dangling_setting_source_and_a_bad_version() {
        let dangling = r#"
            [adapter]
            schema_version = "1.0"
            write_strategy = "anytime"
            [sources]
            cfg = { path = "~/.config/a.toml", format = "toml" }
            [[settings]]
            key = "k"
            source = "nope"
            label = "L"
            type = "bool"
        "#;
        assert!(matches!(
            AdapterManifest::parse(dangling, &home()),
            Err(AdapterError::DanglingSource { .. })
        ));

        let bad_version = r#"
            [adapter]
            schema_version = "9.9"
            write_strategy = "anytime"
        "#;
        assert!(matches!(
            AdapterManifest::parse(bad_version, &home()),
            Err(AdapterError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn refuses_an_unknown_field() {
        // deny_unknown_fields keeps an adapter from carrying anything off-model.
        let extra = r#"
            [adapter]
            schema_version = "1.0"
            write_strategy = "anytime"
            run_script = "evil.sh"
        "#;
        assert!(matches!(
            AdapterManifest::parse(extra, &home()),
            Err(AdapterError::Parse(_))
        ));
    }
}
