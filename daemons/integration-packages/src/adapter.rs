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

use crate::allowlist::{resolve_under_allowlist, AllowlistError, ALLOWED_SUBDIRS};
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
    /// The adapter's sources span more than one app's config subtree, or one of
    /// them globs across apps.
    #[error(
        "an adapter must stay inside ONE app's config subtree; sources reach {first:?} and {second:?}"
    )]
    CrossApp {
        /// The subtree the earlier sources settled on.
        first: String,
        /// The subtree that broke out of it.
        second: String,
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
        self.check_single_app_subtree(home)?;
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

    /// Refuse an adapter whose sources reach into more than one app's config
    /// subtree (E7).
    ///
    /// The allowlist already keeps an adapter inside the user's own config, and
    /// the resolver narrows each individual capability to its source's glob-free
    /// prefix, so no single access can wander sideways. Neither stops an adapter
    /// from simply DECLARING sources in two apps: a Firefox adapter that also
    /// lists `~/.config/chromium/...` gets a legitimate capability for each. This
    /// is the adapter-level bound the other two layers cannot express.
    ///
    /// Derived from the sources rather than declared in a manifest field. Both
    /// were acceptable; derivation wins because it cannot drift from what the
    /// adapter actually touches - a declared subtree is a second thing to keep in
    /// sync, and since an adapter is untrusted community data, it would be a
    /// second thing an author can get wrong or lie about while the real reach is
    /// in the source list either way.
    fn check_single_app_subtree(&self, home: &Path) -> Result<(), AdapterError> {
        let mut settled: Option<String> = None;
        // Sorted so the reported pair is stable regardless of map iteration order.
        let mut paths: Vec<&String> = self.sources.values().map(|s| &s.path).collect();
        paths.sort();
        for path in paths {
            let Some(subtree) = app_subtree(path, home) else {
                // A named file directly under an allowlist root reaches exactly
                // itself, so it belongs to no app subtree and constrains nothing.
                continue;
            };
            match &settled {
                None => settled = Some(subtree),
                Some(first) if *first == subtree => {}
                Some(first) => {
                    return Err(AdapterError::CrossApp {
                        first: first.clone(),
                        second: subtree,
                    })
                }
            }
        }
        Ok(())
    }
}

/// The app subtree a source path sits in: the first component below its
/// allowlist root, or `None` when the path names a file directly in that root
/// (which reaches only itself).
///
/// A glob in that first position - `~/.config/*/prefs` - is NOT `None`: it would
/// reach every app under the root, so it is returned verbatim and will only ever
/// match itself, which makes any second source a cross-app breakout.
fn app_subtree(raw_source_path: &str, home: &Path) -> Option<String> {
    let abs = resolve_under_allowlist(raw_source_path, home).ok()?;
    let relative = ALLOWED_SUBDIRS
        .iter()
        .find_map(|sub| abs.strip_prefix(home.join(sub)).ok().map(|r| r.to_path_buf()))?;
    let mut comps = relative.components();
    let first = comps.next()?.as_os_str().to_string_lossy().into_owned();
    // Nothing after it: the source names a file sitting in the root itself.
    comps.next()?;
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    fn adapter_with(sources: &[(&str, &str)]) -> String {
        let mut t = String::from(
            "[adapter]\nschema_version = \"1.0\"\nwrite_strategy = \"requires_app_closed\"\n\n[sources]\n",
        );
        for (name, path) in sources {
            t.push_str(&format!("{name} = {{ path = \"{path}\", format = \"ini\" }}\n"));
        }
        t
    }

    #[test]
    fn an_adapter_may_not_declare_sources_in_two_apps() {
        // The allowlist admits both paths and the resolver narrows each capability
        // correctly, so nothing below this level objects: a Firefox adapter that
        // also lists chromium simply gets a working capability for each.
        let text = adapter_with(&[
            ("prefs", "~/.config/firefox/prefs.js"),
            ("other", "~/.config/chromium/Preferences"),
        ]);
        // `parse` validates, so the refusal lands there. Sources are checked in
        // path order, so the pair is reported deterministically.
        match AdapterManifest::parse(&text, &home()) {
            Err(AdapterError::CrossApp { first, second }) => {
                assert_eq!((first.as_str(), second.as_str()), ("chromium", "firefox"));
            }
            other => panic!("expected a cross-app refusal, got {other:?}"),
        }
    }

    #[test]
    fn several_sources_inside_one_app_are_fine() {
        let text = adapter_with(&[
            ("a", "~/.config/firefox/prefs.js"),
            ("b", "~/.config/firefox/profiles/*/user.js"),
        ]);
        AdapterManifest::parse(&text, &home()).expect("one app subtree is allowed");
    }

    #[test]
    fn a_glob_in_the_app_position_cannot_be_paired_with_anything() {
        // `~/.config/*/x` reaches every app under the root. It is not treated as
        // "no subtree" - that would let it sit quietly beside a real one.
        let text = adapter_with(&[
            ("wide", "~/.config/*/settings.ini"),
            ("real", "~/.config/firefox/prefs.js"),
        ]);
        assert!(matches!(
            AdapterManifest::parse(&text, &home()),
            Err(AdapterError::CrossApp { .. })
        ));
    }

    #[test]
    fn a_file_directly_in_the_root_reaches_only_itself() {
        // `~/.config/mimeapps.list` is a real pattern and belongs to no app
        // subtree, so it neither claims one nor conflicts with one.
        let text = adapter_with(&[
            ("mime", "~/.config/mimeapps.list"),
            ("app", "~/.config/firefox/prefs.js"),
        ]);
        AdapterManifest::parse(&text, &home()).expect("a root-level file constrains nothing");
    }

    #[test]
    fn two_allowlist_roots_are_still_two_subtrees() {
        let text = adapter_with(&[
            ("a", "~/.mozilla/firefox/prefs.js"),
            ("b", "~/.config/chromium/Preferences"),
        ]);
        assert!(matches!(
            AdapterManifest::parse(&text, &home()),
            Err(AdapterError::CrossApp { .. })
        ));
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
