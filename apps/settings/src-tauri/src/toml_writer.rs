//! Format-preserving TOML writer for hand-edited config files.
//!
//! `compositor.toml` and `shell.toml` are user-authored: people add
//! comments, change formatting, group sections their way. A naive
//! `toml::to_string_pretty(&full_struct)` round-trip would erase all
//! of that on the first Settings-app save. We use `toml_edit` instead
//! to surgically update only the keys the user changed, leaving every
//! comment and blank line intact.
//!
//! The atomic-write contract matches `update_compositor_toml` in the
//! compositor crate: write to `<file>.tmp` then `rename`. POSIX
//! `rename` within the same filesystem is atomic, so a process crash
//! can't leave a half-written config that breaks the next session.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use toml_edit::{DocumentMut, Item};

/// Process-wide guard. Both `compositor.toml` and `shell.toml` go
/// through this single mutex so two settings panels writing
/// concurrently can't interleave their load-modify-write windows.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Apply a closure to the parsed `DocumentMut`, then write atomically.
///
/// Creates the file (and parent dirs) if missing. Returns the patched
/// `DocumentMut` so callers that want to read-back can do so without
/// re-parsing.
///
/// The closure receives a `&mut DocumentMut` so it can do anything
/// `toml_edit` supports — single-key sets, table inserts, array
/// edits, key removal, etc.
pub fn update<F>(path: &Path, patch: F) -> Result<DocumentMut, String>
where
    F: FnOnce(&mut DocumentMut) -> Result<(), String>,
{
    let _guard = WRITE_LOCK
        .lock()
        .map_err(|_| "TOML WRITE_LOCK poisoned".to_string())?;

    let mut doc = read_or_empty(path)?;
    patch(&mut doc)?;
    write_atomic(path, &doc)?;
    Ok(doc)
}

/// Convenience wrapper for the most common case: set
/// `[section].key = value`. Pass a `toml_edit::value(...)` for the
/// value so any TOML-encodable type works (booleans, ints, strings,
/// arrays, inline tables).
///
/// Returns the patched document.
pub fn set_value(
    path: &Path,
    section: &str,
    key: &str,
    value: Item,
) -> Result<DocumentMut, String> {
    update(path, |doc| {
        let table = doc
            .entry(section)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| {
                format!("[{section}] is not a table in {}", path.display())
            })?;
        // Indexed assign instead of `.insert()` because `insert`
        // replaces the entry whole — including its leading comment
        // decor — while `table[key] = value` mutates the value
        // in-place and keeps the user's comment block above the
        // key intact.
        table[key] = value;
        Ok(())
    })
}

/// Read the file as a `DocumentMut`, returning an empty document if
/// the file doesn't exist yet. Errors only on parse failure of an
/// existing file — a missing file is the expected first-save state.
fn read_or_empty(path: &Path) -> Result<DocumentMut, String> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    content
        .parse::<DocumentMut>()
        .map_err(|e| format!("parse {}: {e}", path.display()))
}

/// `<file>.tmp` write + rename. Same pattern desktop-shell uses for
/// `shell.toml` and the compositor for `compositor.state.toml`.
fn write_atomic(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let tmp = with_tmp_extension(path);
    let serialized = doc.to_string();
    std::fs::write(&tmp, serialized.as_bytes())
        .map_err(|e| format!("write tmp {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}

fn with_tmp_extension(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!("{e}.tmp"))
        .unwrap_or_else(|| "tmp".to_string());
    path.with_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests in this module redirect file paths into temp files
    /// directly, so the WRITE_LOCK is the only piece of process-
    /// global state we touch and it's already a Mutex.

    fn temp_file(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("lunaris-toml-writer-test-{name}.toml"));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// User comments and formatting must survive a write that only
    /// touches one key. This is the whole point of toml_edit over
    /// plain `toml::to_string_pretty(&struct)` — anything else is
    /// just a slower full-file write.
    #[test]
    fn set_value_preserves_user_comments() {
        let path = temp_file("preserves-comments");
        let initial = r#"
# User-authored top header.
[layout]
# Inner gap in pixels.
inner_gap = 8
outer_gap = 8

# Trailing comment.
"#;
        std::fs::write(&path, initial).unwrap();

        set_value(
            &path,
            "layout",
            "inner_gap",
            toml_edit::value(12_i64),
        )
        .expect("set");

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# User-authored top header."));
        assert!(written.contains("# Inner gap in pixels."));
        assert!(written.contains("# Trailing comment."));
        assert!(
            written.contains("inner_gap = 12"),
            "value not updated; got:\n{written}"
        );
        assert!(
            written.contains("outer_gap = 8"),
            "unrelated key clobbered; got:\n{written}"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A missing file becomes a fresh document with just the new
    /// section and key. No surprises for first-time settings save.
    #[test]
    fn set_value_creates_missing_file() {
        let path = temp_file("creates-missing");
        let _ = std::fs::remove_file(&path);
        assert!(!path.exists(), "test setup");

        set_value(
            &path,
            "layout",
            "autotile",
            toml_edit::value(true),
        )
        .expect("set");

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[layout]"));
        assert!(written.contains("autotile = true"));

        let _ = std::fs::remove_file(&path);
    }

    /// `update` accepts arbitrary mutations — array insert, table
    /// insert, key removal — through the closure form.
    #[test]
    fn update_can_mutate_arbitrarily() {
        let path = temp_file("arbitrary-mutate");
        std::fs::write(&path, "[a]\nx = 1\n[b]\ny = 2\n").unwrap();

        update(&path, |doc| {
            // Remove key from [a]
            doc["a"].as_table_mut().unwrap().remove("x");
            // Add new key to [b]
            doc["b"]["z"] = toml_edit::value(99_i64);
            // New section
            doc["c"] = toml_edit::table();
            doc["c"]["greeting"] = toml_edit::value("hi");
            Ok(())
        })
        .expect("update");

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("x = 1"), "x not removed: {written}");
        assert!(written.contains("y = 2"));
        assert!(written.contains("z = 99"));
        assert!(written.contains("[c]"));
        assert!(written.contains("greeting = \"hi\""));

        let _ = std::fs::remove_file(&path);
    }

    /// A torn-write (closure errors midway) must NOT clobber the
    /// existing file — the atomic-rename only fires on closure
    /// success. Verifies the abort path leaves the file untouched.
    #[test]
    fn update_closure_error_does_not_write() {
        let path = temp_file("closure-error");
        std::fs::write(&path, "[layout]\ninner_gap = 4\n").unwrap();

        let result = update(&path, |_doc| Err::<(), String>("simulated".into()));
        assert!(result.is_err());

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("inner_gap = 4"),
            "file was modified despite error: {written}"
        );

        let _ = std::fs::remove_file(&path);
    }
}
