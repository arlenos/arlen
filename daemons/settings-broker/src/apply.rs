//! Applying a decided write to the app's `config.toml`.
//!
//! Three properties the plan asks for, each with a reason it is not optional:
//!
//! **Format-preserving.** The file is the user's, and they may have edited it by
//! hand. Re-serialising a parsed document would silently discard their comments
//! and ordering, so the edit goes through `toml_edit` and touches only the key
//! being written.
//!
//! **Atomic, mode-preserving.** Write to a temporary file, fsync it, set the
//! original's permissions on it, then rename. A crash mid-write leaves the old
//! file intact rather than a truncated one, and a config that was deliberately
//! tightened to 0600 does not silently widen to the umask default because the
//! broker rewrote it.
//!
//! **Truthful about change.** Writing the value a key already holds reports NO
//! change. The change signal carries the exact changed key set, and an app
//! live-reloading on it should not be woken for a write that changed nothing.

use std::path::Path;

use toml::Value;
use toml_edit::{DocumentMut, Item};

/// Why an apply failed. Distinct from a refusal: the write was permitted, the
/// filesystem or the file's contents got in the way.
#[derive(Debug)]
pub enum ApplyError {
    /// The config file could not be read or written.
    Io(std::io::Error),
    /// The existing file is not valid TOML, so editing it in place would risk
    /// destroying whatever it actually contains.
    Malformed(String),
    /// A segment of the dotted key collides with a non-table value already in
    /// the file (`a = 1` blocks writing `a.b`).
    PathConflict(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Io(e) => write!(f, "config io: {e}"),
            ApplyError::Malformed(e) => write!(f, "config is not valid TOML: {e}"),
            ApplyError::PathConflict(k) => {
                write!(f, "'{k}' is a value, so it cannot hold a nested key")
            }
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<std::io::Error> for ApplyError {
    fn from(e: std::io::Error) -> Self {
        ApplyError::Io(e)
    }
}

/// Set `key` to `value` in `doc`, creating intermediate tables as needed.
/// Returns whether the document actually changed.
pub fn set_in_document(
    doc: &mut DocumentMut,
    key: &str,
    value: &Value,
) -> Result<bool, ApplyError> {
    let segments: Vec<&str> = key.split('.').collect();
    let (last, parents) = segments.split_last().expect("key is never empty");

    let mut table = doc.as_table_mut();
    for segment in parents {
        // An implicit table renders without its own `[header]` when it only
        // exists to hold the key being written, which keeps a one-key write from
        // adding an empty-looking section to the file.
        let entry = table.entry(segment).or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            Item::Table(t)
        });
        table = entry
            .as_table_mut()
            .ok_or_else(|| ApplyError::PathConflict(segment.to_string()))?;
    }

    let mut new_value = to_edit_value(value);

    // Compare rendered values rather than replacing unconditionally: an
    // unchanged write must not report a change, and must not disturb the file.
    if let Some(existing) = table.get(last) {
        if existing.to_string().trim() == Item::Value(new_value.clone()).to_string().trim() {
            return Ok(false);
        }
        // Carry the existing decor across. A leading comment belongs to the key
        // it sits above, so replacing the item outright would delete the user's
        // note about the very setting being changed. `Table::insert` resets
        // decor, so assign through the index instead and re-attach it.
        if let Item::Value(old) = existing {
            let decor = old.decor().clone();
            *new_value.decor_mut() = decor;
        }
    }
    table[last] = Item::Value(new_value);
    Ok(true)
}

/// Convert a `toml::Value` into the editable representation.
fn to_edit_value(value: &Value) -> toml_edit::Value {
    match value {
        Value::String(s) => s.as_str().into(),
        Value::Integer(i) => (*i).into(),
        Value::Float(f) => (*f).into(),
        Value::Boolean(b) => (*b).into(),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(to_edit_value(item));
            }
            toml_edit::Value::Array(array)
        }
        // Datetimes and tables are not settings types (the schema has no such
        // variant), so rendering through the text form is sufficient and keeps
        // this total rather than panicking on an unreachable case.
        other => other
            .to_string()
            .parse::<toml_edit::Value>()
            .unwrap_or_else(|_| toml_edit::Value::from(other.to_string())),
    }
}

/// Apply a write to the file at `path`, returning whether anything changed.
///
/// A missing file is treated as empty: the first setting an app writes should
/// not require the file to exist already.
pub fn apply_to_file(path: &Path, key: &str, value: &Value) -> Result<bool, ApplyError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(ApplyError::Io(e)),
    };
    let mut doc = existing
        .parse::<DocumentMut>()
        .map_err(|e| ApplyError::Malformed(e.to_string()))?;

    if !set_in_document(&mut doc, key, value)? {
        return Ok(false);
    }
    atomic_write_preserving_mode(path, doc.to_string().as_bytes())?;
    Ok(true)
}

/// Write a document to `path` atomically, keeping its existing permissions.
/// Shared so the migration path writes exactly the way the write path does.
pub fn write_document(path: &Path, doc: &DocumentMut) -> Result<(), ApplyError> {
    atomic_write_preserving_mode(path, doc.to_string().as_bytes())?;
    Ok(())
}

/// Write `bytes` to `path` atomically, keeping the file's existing permissions.
fn atomic_write_preserving_mode(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "config path has no parent")
    })?;
    std::fs::create_dir_all(dir)?;

    // Capture the mode BEFORE writing: after the rename the original is gone.
    let existing_mode = std::fs::metadata(path).ok().map(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode()
    });

    let tmp = path.with_extension("toml.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Some(mode) = existing_mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    }
    std::fs::rename(&tmp, path)?;
    // Durability of the rename itself.
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_config(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    /// The user's comments and unrelated keys must survive a write; they may
    /// have hand-edited this file.
    #[test]
    fn a_write_preserves_comments_and_other_keys() {
        let (_dir, path) = temp_config(
            "# my notes\ntheme = \"dark\"\n\n# keep me\nother = 42\n",
        );
        assert!(apply_to_file(&path, "theme", &Value::String("light".into())).unwrap());

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# my notes"), "{after}");
        assert!(after.contains("# keep me"), "{after}");
        assert!(after.contains("other = 42"), "{after}");
        assert!(after.contains("theme = \"light\""), "{after}");
    }

    /// The change signal carries the changed key set, so a write that changes
    /// nothing must report nothing - otherwise every app live-reloads on a no-op.
    #[test]
    fn writing_an_unchanged_value_reports_no_change() {
        let (_dir, path) = temp_config("theme = \"dark\"\n");
        let before = std::fs::read_to_string(&path).unwrap();

        assert!(!apply_to_file(&path, "theme", &Value::String("dark".into())).unwrap());
        // The file is also left byte-identical, not rewritten identically.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    /// A config deliberately tightened to 0600 must not widen because the broker
    /// rewrote it.
    #[test]
    fn a_write_preserves_the_file_mode() {
        let (_dir, path) = temp_config("theme = \"dark\"\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        assert!(apply_to_file(&path, "theme", &Value::String("light".into())).unwrap());

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode widened to {mode:o}");
    }

    #[test]
    fn a_dotted_key_creates_the_nested_table() {
        let (_dir, path) = temp_config("theme = \"dark\"\n");
        assert!(apply_to_file(&path, "window.width", &Value::Integer(1200)).unwrap());

        let after = std::fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&after).unwrap();
        assert_eq!(parsed["window"]["width"].as_integer(), Some(1200));
    }

    #[test]
    fn a_missing_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert!(apply_to_file(&path, "enabled", &Value::Boolean(true)).unwrap());

        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["enabled"].as_bool(), Some(true));
    }

    /// Editing in place must not proceed against a file we cannot parse: the
    /// safe move is to refuse, not to overwrite whatever is really there.
    #[test]
    fn a_malformed_file_is_refused_and_left_alone() {
        let (_dir, path) = temp_config("this is not = = toml\n");
        let before = std::fs::read_to_string(&path).unwrap();

        let result = apply_to_file(&path, "theme", &Value::String("dark".into()));
        assert!(matches!(result, Err(ApplyError::Malformed(_))), "{result:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    /// `a = 1` cannot also be a table, and silently replacing the user's value
    /// would lose data.
    #[test]
    fn a_scalar_blocking_a_dotted_path_is_refused() {
        let (_dir, path) = temp_config("window = 3\n");
        let result = apply_to_file(&path, "window.width", &Value::Integer(800));
        assert!(matches!(result, Err(ApplyError::PathConflict(_))), "{result:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "window = 3\n");
    }

    #[test]
    fn a_string_list_round_trips() {
        let (_dir, path) = temp_config("");
        let list = Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ]);
        assert!(apply_to_file(&path, "hosts", &list).unwrap());

        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let hosts = parsed["hosts"].as_array().unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].as_str(), Some("a"));
    }

    /// No `.tmp` file may survive a successful write.
    #[test]
    fn the_temporary_file_does_not_linger() {
        let (dir, path) = temp_config("a = 1\n");
        apply_to_file(&path, "a", &Value::Integer(2)).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left {leftovers:?}");
    }
}
