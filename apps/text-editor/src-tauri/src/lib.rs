// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The text editor's host process.
//!
//! The frontend has existed for some time and the host did not, which meant the
//! app could be rendered but never launched - it was the last one in the tree in
//! that state. This is the launchable half: a window, the shell plugin the other
//! apps use for theme and locale, and the file commands an editor cannot do
//! without.
//!
//! What is deliberately NOT here yet: the lens (`provenance_of`, `related_of`,
//! `project_of`) and the gated AI edit (`ai_edit` and its accept/reject/undo).
//! Those are reads against the knowledge daemon and the AI engine, and the
//! frontend already renders an honest "not available" state for each, so the
//! editor is useful before they land rather than lying about them.

mod lens;

use std::path::{Path, PathBuf};

/// A file the editor opened: its path, its text, and whether it is new.
#[derive(Debug, serde::Serialize)]
pub struct OpenedFile {
    /// The absolute path, as the editor should display and later save it.
    pub path: String,
    /// The file's contents.
    pub text: String,
}

/// Reject a path that is not absolute.
///
/// The frontend passes what the user picked, and every later step - the save,
/// the lens lookup, the KG's own file identity - keys off an absolute path. A
/// relative one would resolve against the host's working directory, which is
/// wherever the launcher happened to start it, and that is not a place the user
/// chose.
fn absolute(path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("{path}: not an absolute path"));
    }
    Ok(p.to_path_buf())
}

/// Read a text file for editing.
///
/// Binary content is refused rather than shown: this editor writes back what it
/// renders, so a file it cannot faithfully round-trip through a string is one it
/// must not open at all. The check is UTF-8 validity, which is the same property
/// the save relies on.
#[tauri::command]
fn editor_open(path: String) -> Result<OpenedFile, String> {
    let p = absolute(&path)?;
    let bytes = std::fs::read(&p).map_err(|e| format!("{path}: {e}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{path}: not UTF-8 text, so this editor will not open it"))?;
    Ok(OpenedFile { path, text })
}

/// Write the edited text back.
///
/// Nothing calls this yet, and that is deliberate rather than forgotten: the
/// canvas is a renderer, not an editing surface (`Canvas.svelte` says so in its
/// own header - the incremental highlighting engine that would make it editable
/// is separate work). A Save that reaches a backend from a surface where nothing
/// can be typed would be the same shape as the fixtures fixed all week: an
/// affordance that implies a capability the app does not have. This is the half
/// that will be correct when the other half exists.
///
/// Writes to a sibling temporary file and renames over the original, so an
/// interrupted save leaves the previous contents intact rather than a truncated
/// file. The rename is atomic within a filesystem; the temp file is created
/// beside the target for exactly that reason.
#[tauri::command]
fn editor_save(path: String, text: String) -> Result<(), String> {
    let p = absolute(&path)?;
    let dir = p.parent().ok_or_else(|| format!("{path}: has no parent directory"))?;
    let tmp = dir.join(format!(
        ".{}.arlen-save",
        p.file_name().and_then(|n| n.to_str()).unwrap_or("untitled")
    ));
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{path}: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| {
        // Leave nothing behind on a failed rename: the temp file is ours.
        let _ = std::fs::remove_file(&tmp);
        format!("{path}: {e}")
    })
}

/// The file path the editor was launched with (`arlen-text-editor <path>`, or the
/// `.desktop` `Exec=<bin> %f` when opened from the file manager). `None` when
/// launched bare, which is the demo-document path.
struct InitialFile(Option<String>);

/// The path the editor was opened on, for the frontend to load on mount.
#[tauri::command]
fn initial_file(state: tauri::State<'_, InitialFile>) -> Option<String> {
    state.0.clone()
}

/// Run the editor.
pub fn run() {
    env_logger::init();
    // The first non-flag argument is the file to open. Same rule as the viewers,
    // so `%f` from a desktop entry lands the same way in both.
    let initial = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(InitialFile(initial))
        .invoke_handler(tauri::generate_handler![
            editor_open,
            editor_save,
            initial_file,
            lens::provenance_of
        ])
        .run(tauri::generate_context!())
        .expect("error while running the text editor");
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: the `#[tauri::command]` macro generates helper items
    // beside each function, and a glob import pulls those in twice. The commands
    // themselves are module-private for the same family of reason - on tauri
    // 2.11.5 a `pub` command re-exports its own generated macro and the crate
    // stops compiling - so the tests reach them as siblings.
    use super::{absolute, editor_open, editor_save};

    #[test]
    fn a_relative_path_is_refused() {
        assert!(absolute("notes.txt").is_err());
        assert!(absolute("./notes.txt").is_err());
        assert!(absolute("/tmp/notes.txt").is_ok());
    }

    #[test]
    fn a_non_utf8_file_is_refused_rather_than_shown() {
        let dir = std::env::temp_dir().join(format!("arlen-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("binary.bin");
        std::fs::write(&f, [0xff, 0xfe, 0x00]).unwrap();
        let e = editor_open(f.to_string_lossy().into_owned()).unwrap_err();
        assert!(e.contains("not UTF-8"), "{e}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_save_replaces_the_file_and_leaves_no_temp_behind() {
        let dir = std::env::temp_dir().join(format!("arlen-editor-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("notes.md");
        std::fs::write(&f, "before").unwrap();
        let path = f.to_string_lossy().into_owned();
        editor_save(path.clone(), "after".into()).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "after");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".arlen-save"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
