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
//! What was deliberately NOT here: the lens's `related_of`. It is answered now -
//! promotion records `LINKS_TO` from a markdown document's own references, so the
//! backlinks have a real edge to traverse. The old note follows for the reasoning
//! that kept it out (see `lens.rs` - the
//! graph holds no file-to-file edge, so "backlinks" needs a meaning before it can
//! have a query) and the gated AI edit (`ai_edit` and its accept/reject/undo).
//! `provenance_of` and `project_of` are answered.
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
    /// What the file looked like when it was read, to be handed back at save.
    /// See [`stamp`].
    pub stamp: String,
}

/// The marker a save returns when the file changed underneath the editor.
///
/// A token rather than a sentence: the wording belongs to the page, where it is
/// translated.
pub const CHANGED_ON_DISK: &str = "file-changed-on-disk";

/// A cheap description of a file's current contents.
///
/// Modification time and length, not a hash of the contents: reading a large
/// file again on every save to prove it has not moved costs more than the
/// problem, and this catches every case a person actually meets - another
/// editor's save, a `git checkout`, a sync writing over it. Two writes inside
/// one filesystem timestamp tick with identical length would slip through, which
/// is a race a human hand cannot produce and a machine writing the file you are
/// editing has already lost.
fn stamp(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        // No file: an unsaved new document has nothing to be changed out from
        // under it, and the empty stamp compares equal to the next one.
        return String::new();
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{modified}:{}", meta.len())
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
/// Why a file did not open, as a word rather than a sentence.
///
/// Three outcomes, and the middle one is ordinary rather than technical: opening
/// a picture or a binary in a text editor is a thing people do by accident. It
/// used to reach the window as "not UTF-8 text, so this editor will not open it",
/// an English sentence built here, under a title the catalogue had translated.
/// `why` survives on the first because the filesystem's own words name the path
/// and the reason.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum OpenProblem {
    /// The path is relative, so there is nothing to resolve it against.
    NotAbsolute,
    /// The file would not read.
    Unreadable { why: String },
    /// It is not text this editor can round-trip, so opening it would risk
    /// writing back something else.
    NotText,
}

#[tauri::command]
fn editor_open(path: String) -> Result<OpenedFile, OpenProblem> {
    let p = absolute(&path).map_err(|_| OpenProblem::NotAbsolute)?;
    let bytes = std::fs::read(&p).map_err(|e| OpenProblem::Unreadable { why: e.to_string() })?;
    let text = String::from_utf8(bytes).map_err(|_| OpenProblem::NotText)?;
    let stamp = stamp(&p);
    Ok(OpenedFile { path, text, stamp })
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
///
/// REFUSES A LOST UPDATE. `seen` is the stamp the editor got when it opened or
/// last saved the file; if the file no longer matches it, something else has
/// written it since and this save would silently destroy that. The refusal
/// carries [`CHANGED_ON_DISK`] so the page can say what happened and let the
/// person choose, and `force` is that choice made deliberately. Passing an empty
/// `seen` means the caller never read the file, which only a new document does.
#[tauri::command]
fn editor_save(path: String, text: String, seen: Option<String>, force: Option<bool>) -> Result<String, String> {
    let p = absolute(&path)?;
    if !force.unwrap_or(false) {
        if let Some(seen) = seen.filter(|s| !s.is_empty()) {
            if stamp(&p) != seen {
                return Err(CHANGED_ON_DISK.to_string());
            }
        }
    }
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
    })?;
    // The stamp of what was just written, so the next save compares against this
    // save rather than against the state at open - otherwise the second save of
    // a session always looks like somebody else's change.
    Ok(stamp(&p))
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
    // This app at info, dependencies at warn, and both halves are a fix.
    //
    // A bare `env_logger::init()` defaults to `error`, so every `log::info!`
    // and `log::warn!` here produced nothing: the app was mute in the journal.
    // That is the failure that made the boot consent hang so hard to find -
    // the component in the middle could not be heard - and it was true of four
    // apps at once.
    //
    // Dependencies stay at warn rather than being swept up to info with it,
    // because zbus logs D-Bus handshake frames WITH their message bytes, and a
    // message body is user content: paths, queries, notification text. At info
    // that lands in a journal no capability grant covers. `RUST_LOG=zbus=trace`
    // still gets it, deliberately.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,arlen_text_editor_lib=info"),
    )
    .init();
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
            lens::provenance_of,
            lens::project_of,
            lens::related_of
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
    use super::{absolute, editor_open, editor_save, CHANGED_ON_DISK};

    /// The lost update, which is the reason any of this exists: open a file,
    /// something else writes it, and a save that goes through silently destroys
    /// the other change. Every editor a person has used guards this; ours wrote
    /// straight over it until 19 August.
    #[test]
    fn a_save_is_refused_when_the_file_changed_underneath_it() {
        let dir = std::env::temp_dir().join(format!("arlen-editor-clobber-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("shared.md");
        std::fs::write(&f, "as opened").unwrap();
        let path = f.to_string_lossy().into_owned();
        let opened = editor_open(path.clone()).unwrap();

        // Somebody else's save. The sleep is the filesystem's timestamp
        // granularity, not a race in the code: two writes inside one tick with
        // the same length are indistinguishable by design, and the test would be
        // asserting the limitation rather than the guard.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "somebody else's work").unwrap();

        let refused = editor_save(path.clone(), "mine".into(), Some(opened.stamp.clone()), None)
            .expect_err("a save over someone else's change must be refused");
        assert_eq!(refused, CHANGED_ON_DISK);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "somebody else's work",
            "the refusal must leave the other change intact"
        );

        // And the person can still decide to win.
        editor_save(path.clone(), "mine".into(), Some(opened.stamp), Some(true)).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "mine");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A second save in one session must not look like somebody else's change,
    /// which it would if the stamp still described the state at open.
    #[test]
    fn saving_twice_in_a_row_is_not_mistaken_for_a_foreign_write() {
        let dir = std::env::temp_dir().join(format!("arlen-editor-twice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("notes.md");
        std::fs::write(&f, "one").unwrap();
        let path = f.to_string_lossy().into_owned();
        let opened = editor_open(path.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let after_first = editor_save(path.clone(), "two".into(), Some(opened.stamp), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        editor_save(path.clone(), "three".into(), Some(after_first), None)
            .expect("the second save compares against the first, not against the open");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "three");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file that did not exist when the editor started has nothing to be
    /// clobbered, so a first save must not be refused.
    #[test]
    fn a_new_file_saves_without_a_stamp() {
        let dir = std::env::temp_dir().join(format!("arlen-editor-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("fresh.md");
        let path = f.to_string_lossy().into_owned();
        editor_save(path, "first".into(), Some(String::new()), None).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "first");
        std::fs::remove_dir_all(&dir).ok();
    }

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
        editor_save(path.clone(), "after".into(), None, None).unwrap();
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
