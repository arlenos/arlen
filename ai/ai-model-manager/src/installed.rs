//! Enumeration of installed local models.
//!
//! The Models hub needs the daemon to answer "which models are on this machine",
//! not just the active one, so a downloaded or baked GGUF is selectable. This
//! module scans the two model directories - the per-user store
//! ([`crate::download::model_store_dir`], where a consented download lands) and the
//! system store ([`system_model_dir`], where the baked offline default lives) - and
//! returns the `*.gguf` files it finds with their basic metadata. Richer GGUF
//! header metadata (parameter count, context length) is a follow-up; catalogue
//! matching by file name gives the display name meanwhile.

use std::path::{Path, PathBuf};

/// Where an installed model lives, which decides whether it is deletable: the
/// baked `System` model is undeletable, a `User` download is the user's to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelLocation {
    /// The per-user store (`~/.local/share/arlen/models`), a consented download.
    User,
    /// The system store (`/usr/share/arlen/models`), the baked offline default.
    System,
}

/// One installed GGUF model found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    /// The GGUF file name (e.g. `Llama-3.2-1B-Instruct-Q4_K_M.gguf`).
    pub file_name: String,
    /// The absolute path to the file.
    pub path: PathBuf,
    /// The file size in bytes.
    pub size_bytes: u64,
    /// Where it lives (which decides deletability).
    pub location: ModelLocation,
}

/// The system model store: `$ARLEN_SYSTEM_MODELS_DIR` (tests/dev) else
/// `/usr/share/arlen/models`. The baked offline default (Llama-3.2-1B) lives here.
pub fn system_model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ARLEN_SYSTEM_MODELS_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    PathBuf::from("/usr/share/arlen/models")
}

/// Scan one directory for `*.gguf` models, tagging each with `location`. A missing
/// or unreadable directory yields an empty list (not an error): a fresh machine
/// simply has no user downloads. Results are sorted by file name for a stable
/// order. Non-files, non-`.gguf`, and unreadable entries are skipped.
pub fn scan_model_dir(dir: &Path, location: ModelLocation) -> Vec<InstalledModel> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut models: Vec<InstalledModel> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_gguf = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
            if !is_gguf {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let file_name = path.file_name()?.to_str()?.to_string();
            Some(InstalledModel {
                file_name,
                path,
                size_bytes: meta.len(),
                location,
            })
        })
        .collect();
    models.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    models
}

/// Every installed model across the user and system stores. A user download and a
/// system model with the same file name collapse to the user one (a user copy
/// shadows the baked default); the result is sorted by file name.
pub fn installed_models() -> Vec<InstalledModel> {
    let mut out: Vec<InstalledModel> = Vec::new();
    if let Some(user_dir) = crate::download::model_store_dir() {
        out.extend(scan_model_dir(&user_dir, ModelLocation::User));
    }
    for model in scan_model_dir(&system_model_dir(), ModelLocation::System) {
        if !out.iter().any(|m| m.file_name == model.file_name) {
            out.push(model);
        }
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, bytes: &[u8]) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }

    #[test]
    fn scan_finds_gguf_files_and_skips_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "b-model.gguf", b"bbbb");
        write_file(tmp.path(), "a-model.gguf", b"aa");
        write_file(tmp.path(), "notes.txt", b"ignore me");
        write_file(tmp.path(), "config.json", b"{}");

        let found = scan_model_dir(tmp.path(), ModelLocation::User);
        assert_eq!(found.len(), 2, "only the two .gguf files");
        // Sorted by file name.
        assert_eq!(found[0].file_name, "a-model.gguf");
        assert_eq!(found[0].size_bytes, 2);
        assert_eq!(found[1].file_name, "b-model.gguf");
        assert_eq!(found[1].size_bytes, 4);
        assert!(found.iter().all(|m| m.location == ModelLocation::User));
    }

    #[test]
    fn scan_of_a_missing_dir_is_empty_not_an_error() {
        let missing = PathBuf::from("/nonexistent/arlen/models/xyz");
        assert!(scan_model_dir(&missing, ModelLocation::System).is_empty());
    }

    #[test]
    fn a_user_download_shadows_the_baked_system_model_of_the_same_name() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        write_file(user.path(), "Llama-3.2-1B.gguf", b"user copy");
        write_file(system.path(), "Llama-3.2-1B.gguf", b"baked");
        write_file(system.path(), "Baked-Only.gguf", b"only in system");

        // Compose the dedup logic directly (installed_models reads real dirs).
        let mut out = scan_model_dir(user.path(), ModelLocation::User);
        for m in scan_model_dir(system.path(), ModelLocation::System) {
            if !out.iter().any(|x| x.file_name == m.file_name) {
                out.push(m);
            }
        }
        out.sort_by(|a, b| a.file_name.cmp(&b.file_name));

        assert_eq!(out.len(), 2);
        let llama = out.iter().find(|m| m.file_name == "Llama-3.2-1B.gguf").unwrap();
        assert_eq!(llama.location, ModelLocation::User, "user copy shadows baked");
        assert_eq!(llama.size_bytes, 9);
        assert!(out.iter().any(|m| m.file_name == "Baked-Only.gguf"));
    }
}
