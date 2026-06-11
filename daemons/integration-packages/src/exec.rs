//! The adapter executor: read a setting's live value and prepare a verified edit
//! (integration-packages-plan.md IP-R2/R3).
//!
//! [`crate::adapter`] parses and confines the adapter; [`crate::resolve`] turns a
//! source glob into concrete files; [`crate::write`] decides WHETHER a write may
//! happen now. This module is the interpreter actually doing the adapter's work
//! over one resolved source file, through `arlen-config-format`. It is the glue
//! the Settings render builds on, and it never writes the file itself: the
//! privileged Settings app owns that I/O. The executor only
//!
//! - reads the current value to display ([`read_setting`], through the S18-B
//!   parse sandbox, since the content is untrusted), and
//! - produces the format-preserving, read-after-write-verified candidate text for
//!   an edit ([`prepare_edit`] / [`prepare_remove`]).
//!
//! Both reject fail-closed: a `readonly` setting is never written, and a value
//! whose type disagrees with the setting's declared type is refused rather than
//! coerced.

use crate::adapter::{SettingSpec, SettingType, SourceSpec};
use arlen_config_format::confined::{read_confined, read_and_parse_confined, ConfinedError};
use arlen_config_format::{checked_remove, checked_set, handler_for, ConfigValue, EditError};
use cap_std::fs::Dir;
use std::path::Path;

/// Why reading or editing a setting through the adapter failed.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The confined read, the S18-B sandbox strip, or the parse failed.
    #[error("read: {0}")]
    Read(#[from] ConfinedError),
    /// The format-preserving edit or its read-after-write self-check failed.
    #[error("edit: {0}")]
    Edit(#[from] EditError),
    /// The file holds bytes that are not valid UTF-8, so it cannot be edited as
    /// text (the format handlers all operate on UTF-8 source).
    #[error("file is not valid UTF-8")]
    NotUtf8,
    /// The setting is declared `readonly`: it may be displayed, never written.
    #[error("setting {key:?} is read-only")]
    ReadOnly {
        /// The setting key.
        key: String,
    },
    /// The new value's type disagrees with the setting's declared type (a `bool`
    /// setting cannot be set to a string, etc.): refuse rather than coerce.
    #[error("setting {key:?} expects a {expected:?} value")]
    TypeMismatch {
        /// The setting key.
        key: String,
        /// The declared type the value had to match.
        expected: SettingType,
    },
}

/// Whether `value` is a legal payload for a setting of `ty`.
///
/// [`ConfigValue::Opaque`] is never a caller-set value (it is only what a handler
/// reports for an existing non-scalar), so it matches no declared type; an `enum`
/// member is carried as text, so [`SettingType::Enum`] accepts a string.
fn value_matches_type(ty: SettingType, value: &ConfigValue) -> bool {
    matches!(
        (ty, value),
        (SettingType::String, ConfigValue::String(_))
            | (SettingType::Enum, ConfigValue::String(_))
            | (SettingType::Int, ConfigValue::Int(_))
            | (SettingType::Bool, ConfigValue::Bool(_))
            | (SettingType::Float, ConfigValue::Float(_))
    )
}

/// Read a setting's current value from a resolved source file, for Settings to
/// display.
///
/// Reads the file through the cap-std capability `dir` (so the read cannot escape
/// the confinement root, and an oversize file is refused not walked), strips it to
/// inert text in the S18-B parse sandbox at `sandbox_bin`, then parses it with the
/// source's format handler and looks `setting.key` up. Returns `None` when the key
/// is absent, so Settings can fall back to the spec default.
pub fn read_setting(
    dir: &Dir,
    rel: impl AsRef<Path>,
    sandbox_bin: &Path,
    source: &SourceSpec,
    setting: &SettingSpec,
) -> Result<Option<ConfigValue>, ExecError> {
    let model = read_and_parse_confined(dir, rel, sandbox_bin, source.format.to_format())?;
    Ok(model.get(&setting.key).cloned())
}

/// Read a resolved source file as editable UTF-8 text, through the cap-std
/// capability `dir`.
///
/// The edit path ([`prepare_edit`] / [`prepare_remove`]) operates on the file's
/// real text, not a sandbox-stripped read, because a format-preserving edit must
/// keep the exact bytes (stripping would corrupt comments and layout). The
/// untrusted-content risk on the edit is contained differently: the read is
/// capability-confined and size-capped here, and the edit's read-after-write
/// self-check rejects any collateral change. Non-UTF-8 content is refused, since
/// the handlers all parse UTF-8 source.
pub fn read_text_confined(dir: &Dir, rel: impl AsRef<Path>) -> Result<String, ExecError> {
    let bytes = read_confined(dir, rel)?;
    String::from_utf8(bytes).map_err(|_| ExecError::NotUtf8)
}

/// Produce the format-preserving candidate text for setting `setting` to
/// `new_value` in `current_text` (which the caller holds, e.g. via
/// [`read_text_confined`]).
///
/// Runs the self-checked [`checked_set`], so the returned text is verified to
/// carry exactly the edit and nothing else; the privileged caller then writes it.
/// Refuses a `readonly` setting and a type-mismatched value before touching the
/// text.
pub fn prepare_edit(
    current_text: &str,
    source: &SourceSpec,
    setting: &SettingSpec,
    new_value: &ConfigValue,
) -> Result<String, ExecError> {
    if setting.readonly {
        return Err(ExecError::ReadOnly {
            key: setting.key.clone(),
        });
    }
    if !value_matches_type(setting.ty, new_value) {
        return Err(ExecError::TypeMismatch {
            key: setting.key.clone(),
            expected: setting.ty,
        });
    }
    let handler = handler_for(source.format.to_format());
    Ok(checked_set(
        handler.as_ref(),
        current_text,
        &setting.key,
        new_value,
    )?)
}

/// Produce the candidate text that removes `setting`'s key from `current_text`,
/// resetting it to the app's own default.
///
/// Runs the self-checked [`checked_remove`] (an absent key is a passing no-op).
/// Refuses a `readonly` setting before touching the text.
pub fn prepare_remove(
    current_text: &str,
    source: &SourceSpec,
    setting: &SettingSpec,
) -> Result<String, ExecError> {
    if setting.readonly {
        return Err(ExecError::ReadOnly {
            key: setting.key.clone(),
        });
    }
    let handler = handler_for(source.format.to_format());
    Ok(checked_remove(handler.as_ref(), current_text, &setting.key)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{FormatName, InstanceStrategy};
    use cap_std::ambient_authority;

    fn source(format: FormatName) -> SourceSpec {
        SourceSpec {
            path: "~/.config/app/config.toml".to_string(),
            format,
            instance_strategy: InstanceStrategy::Ask,
        }
    }

    fn setting(key: &str, ty: SettingType, readonly: bool) -> SettingSpec {
        SettingSpec {
            key: key.to_string(),
            source: "cfg".to_string(),
            label: "L".to_string(),
            ty,
            default: None,
            section: None,
            verify: false,
            readonly,
        }
    }

    #[test]
    fn value_matches_type_is_strict_and_enum_is_text() {
        assert!(value_matches_type(
            SettingType::String,
            &ConfigValue::String("x".into())
        ));
        assert!(value_matches_type(
            SettingType::Enum,
            &ConfigValue::String("dark".into())
        ));
        assert!(value_matches_type(SettingType::Int, &ConfigValue::Int(5)));
        assert!(value_matches_type(SettingType::Bool, &ConfigValue::Bool(true)));
        assert!(value_matches_type(SettingType::Float, &ConfigValue::Float(1.5)));
        // Cross-type and Opaque are refused.
        assert!(!value_matches_type(SettingType::Bool, &ConfigValue::Int(1)));
        assert!(!value_matches_type(
            SettingType::Int,
            &ConfigValue::String("5".into())
        ));
        assert!(!value_matches_type(SettingType::Float, &ConfigValue::Int(1)));
        assert!(!value_matches_type(SettingType::String, &ConfigValue::Opaque));
    }

    #[test]
    fn prepare_edit_produces_a_verified_format_preserving_candidate() {
        let original = "# user config\nname = \"old\"\nport = 8080\n";
        let out = prepare_edit(
            original,
            &source(FormatName::Toml),
            &setting("port", SettingType::Int, false),
            &ConfigValue::Int(9090),
        )
        .unwrap();
        // The edit took, the comment and the untouched key survived.
        assert!(out.contains("port = 9090"));
        assert!(out.contains("# user config"));
        assert!(out.contains("name = \"old\""));
    }

    #[test]
    fn prepare_edit_refuses_a_readonly_setting() {
        let err = prepare_edit(
            "name = \"x\"\n",
            &source(FormatName::Toml),
            &setting("name", SettingType::String, true),
            &ConfigValue::String("y".into()),
        )
        .unwrap_err();
        assert!(matches!(err, ExecError::ReadOnly { .. }));
    }

    #[test]
    fn prepare_edit_refuses_a_type_mismatched_value() {
        let err = prepare_edit(
            "enabled = true\n",
            &source(FormatName::Toml),
            &setting("enabled", SettingType::Bool, false),
            &ConfigValue::String("yes".into()),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ExecError::TypeMismatch {
                expected: SettingType::Bool,
                ..
            }
        ));
    }

    #[test]
    fn prepare_remove_resets_to_default_and_refuses_readonly() {
        let out = prepare_remove(
            "name = \"x\"\nport = 8080\n",
            &source(FormatName::Toml),
            &setting("port", SettingType::Int, false),
        )
        .unwrap();
        assert!(!out.contains("port"));
        assert!(out.contains("name = \"x\""));

        let err = prepare_remove(
            "name = \"x\"\n",
            &source(FormatName::Toml),
            &setting("name", SettingType::String, true),
        )
        .unwrap_err();
        assert!(matches!(err, ExecError::ReadOnly { .. }));
    }

    #[test]
    fn read_text_confined_reads_within_the_capability_and_refuses_escape() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), b"name = \"arlen\"\n").unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let text = read_text_confined(&dir, "config.toml").unwrap();
        assert_eq!(text, "name = \"arlen\"\n");
        // A traversal and an absolute path are refused by the capability root.
        assert!(read_text_confined(&dir, "../escape").is_err());
        assert!(read_text_confined(&dir, "/etc/hostname").is_err());
    }

    #[test]
    fn read_text_confined_refuses_non_utf8() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bad"), [0xff, 0xfe, 0x00]).unwrap();
        let dir = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        assert!(matches!(
            read_text_confined(&dir, "bad").unwrap_err(),
            ExecError::NotUtf8
        ));
    }
}
