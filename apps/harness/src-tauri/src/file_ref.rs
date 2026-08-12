//! Clickable file references - the chat file-pill backend seams (surface 1 of
//! `harness-file-refs-and-diffs-plan.md`; the contract arlen-ui's `fileRefs.ts`
//! action invokes).
//!
//! The agent names a file as a markdown link with the `arlenfile://` scheme; the
//! frontend upgrades each anchor into a pill and drives these commands with the
//! plain absolute path. All of them act with the USER's authority through the
//! desktop opener / file manager (the powerbox principle) - never through the AI
//! daemon, so a click never touches the agent's capability-scoped read ledger.
//! Paths are passed as single arguments (no shell), so they carry no injection
//! risk.
//!
//! `fileref_open_with` is the powerbox form: `OpenURI` with `ask: true`, which
//! makes the DESKTOP show its chooser rather than the harness growing one. This
//! header used to say it was not built because the correct shape needed
//! file-descriptor passing over D-Bus - that is `OpenFile`, the fd variant. The
//! URI variant takes a `file://` URI, the portal plugin already permits that
//! scheme and already carries the `ask` option, so the blocker described here had
//! dissolved. The menu item shipped in the meantime and threw on every click.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// How one file pill should render, returned by [`fileref_resolve`]. `path`
/// echoes the input so the frontend correlates each result to its pill in a
/// batch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileRefState {
    /// The input path, echoed for correlation.
    pub path: String,
    /// Whether the path resolves to an existing filesystem object. A broken
    /// symlink, a deleted file, or a never-existed path reads as `false` (the
    /// pill renders muted with no click target, copies-only menu).
    pub resolvable: bool,
    /// The MIME type from `xdg-mime query filetype` (the same resolution that
    /// selects the opener), or absent when unresolvable / a directory / no MIME
    /// database answers. The frontend prefers it for the icon, falling back to
    /// the extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

/// Query the system MIME database for `path`'s type via `xdg-mime`. `None` when
/// `xdg-mime` is absent, fails, or answers empty.
///
/// Takes a `&Path` that the caller has already canonicalized, and passes its
/// `OsStr` through unconverted. Both parts matter: `xdg-mime` reads a
/// leading-dash argument as an option (`xdg-mime query filetype -x.txt` answers
/// "unexpected option"), so a relative `-notes.txt` would come back untyped; and
/// a lossy string conversion would ask the database about a different file
/// whenever a name is not valid UTF-8. This took a `&str` built with
/// `to_string_lossy` and was safe only because its one caller happened to
/// canonicalize first.
fn query_mime(path: &Path) -> Option<String> {
    let output = Command::new("xdg-mime")
        .args([
            std::ffi::OsStr::new("query"),
            std::ffi::OsStr::new("filetype"),
            path.as_os_str(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mime = String::from_utf8(output.stdout).ok()?;
    let trimmed = mime.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Resolve a batch of file references (one call per chat message) into their
/// pill render state: resolvable + MIME. Canonicalises each path to decide
/// existence (following symlinks; a dangling one reads unresolvable); a
/// directory gets no MIME (its opener is the file manager). Pure over the
/// filesystem so the per-path shaping is unit-tested.
#[tauri::command]
pub async fn fileref_resolve(paths: Vec<String>) -> Vec<FileRefState> {
    paths.into_iter().map(|path| resolve_one(&path)).collect()
}

/// Resolve a single path's pill state. Split out so the batch command is a thin
/// map and the per-path logic is unit-tested.
fn resolve_one(path: &str) -> FileRefState {
    match std::fs::canonicalize(path) {
        Ok(real) => {
            let mime = if real.is_dir() {
                None
            } else {
                query_mime(&real)
            };
            FileRefState {
                path: path.to_string(),
                resolvable: true,
                mime,
            }
        }
        Err(_) => FileRefState {
            path: path.to_string(),
            resolvable: false,
            mime: None,
        },
    }
}

/// Open `path` with the user's default handler via `xdg-open` (which routes
/// through `xdg-desktop-portal` on a portal desktop) - the powerbox seam. The
/// authority is the user's click, NOT the AI daemon. Refuses an unresolvable
/// path (the fail-safe backstop behind the muted-pill no-click-target).
#[tauri::command]
pub async fn fileref_open(path: String) -> Result<(), String> {
    let real = std::fs::canonicalize(&path).map_err(|_| format!("not found: {path}"))?;
    open_via_launch(&real.to_string_lossy()).await
}

/// Reveal `path` in the file manager (select it in its containing folder) - the
/// "Reveal in Files" action. Uses the freedesktop `FileManager1.ShowItems` D-Bus
/// interface (select-in-folder), falling back to opening the containing folder
/// via `xdg-open` when no file manager owns that name. Refuses an unresolvable
/// path.
#[tauri::command]
pub async fn fileref_reveal(path: String) -> Result<(), String> {
    let real = std::fs::canonicalize(&path).map_err(|_| format!("not found: {path}"))?;
    let real_str = real.to_string_lossy().into_owned();
    if reveal_via_filemanager1(&real_str).await.is_ok() {
        return Ok(());
    }
    // Fallback: open the containing folder (no selection) with the default opener.
    let parent = real.parent().unwrap_or(&real);
    open_via_launch(&parent.to_string_lossy()).await
}

/// Open an absolute path through the shell's launch socket.
///
/// Both callers canonicalize first, so the argument is absolute by the time it
/// arrives; the check is here anyway because that is a property of the callers
/// rather than of this function, and it is the callers that could change.
async fn open_via_launch(arg: &str) -> Result<(), String> {
    if !arg.starts_with('/') {
        return Err(format!("not an absolute path: {arg}"));
    }
    match arlen_launch_contract::open_path(arg)
        .await
        .map_err(|e| e.to_string())?
    {
        arlen_launch_contract::LaunchOutcome::Started { .. } => Ok(()),
        // The outcome is reported rather than flattened: "nothing opens this
        // kind of file" is the answer a person can act on, and a spawn could
        // never tell them that.
        other => Err(format!("{other:?}")),
    }
}

/// Call `org.freedesktop.FileManager1.ShowItems` on the session bus to select
/// `path` in the file manager. `path` is a canonical absolute path; it is
/// wrapped as a `file://` URI. Errs when no file manager owns the name or the
/// call fails (the caller falls back to opening the folder).
async fn reveal_via_filemanager1(path: &str) -> Result<(), String> {
    let uri = format!("file://{path}");
    let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
    conn.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        &(vec![uri], ""),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_one_marks_a_real_file_resolvable_and_echoes_the_input_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, b"hi").unwrap();
        let input = file.to_string_lossy().into_owned();

        let r = resolve_one(&input);
        assert!(r.resolvable);
        assert_eq!(r.path, input, "the input path is echoed for correlation");
    }

    #[test]
    fn resolve_one_marks_a_missing_path_unresolvable_with_no_mime() {
        let r = resolve_one("/no/such/path/ghost.md");
        assert!(!r.resolvable);
        assert_eq!(r.mime, None);
        assert_eq!(r.path, "/no/such/path/ghost.md");
    }

    #[test]
    fn resolve_one_treats_a_directory_as_resolvable_without_a_mime() {
        let dir = tempfile::tempdir().unwrap();
        let r = resolve_one(&dir.path().to_string_lossy());
        assert!(r.resolvable);
        assert_eq!(r.mime, None, "a directory opens in the file manager, no MIME icon");
    }

    #[test]
    fn resolve_one_reads_a_broken_symlink_as_unresolvable() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("gone"), &link).unwrap();
        assert!(!resolve_one(&link.to_string_lossy()).resolvable);
    }

    #[tokio::test]
    async fn fileref_resolve_maps_a_batch_preserving_order() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("a.txt");
        std::fs::write(&real, b"x").unwrap();
        let real_s = real.to_string_lossy().into_owned();

        let out = fileref_resolve(vec![real_s.clone(), "/missing/b.md".to_string()]).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path, real_s);
        assert!(out[0].resolvable);
        assert!(!out[1].resolvable);
        assert_eq!(out[1].path, "/missing/b.md");
    }

    #[tokio::test]
    async fn opening_or_revealing_a_missing_path_is_refused() {
        assert!(fileref_open("/no/such/ghost.md".to_string())
            .await
            .unwrap_err()
            .contains("not found"));
        assert!(fileref_reveal("/no/such/ghost.md".to_string())
            .await
            .unwrap_err()
            .contains("not found"));
    }
}

/// The pill's "Open with...": the desktop's own application chooser.
///
/// `ask: true` is the whole request - it tells the portal to prompt rather than
/// dispatch to the default handler, which is what makes this the powerbox form.
/// The harness never learns which application was picked and never needs to: the
/// user choosing IS the authorisation, and an app that had to be told the answer
/// would be an app that could have asked for it without them.
#[tauri::command]
pub async fn fileref_open_with(path: String) -> Result<(), String> {
    // Canonical first, like the other two: the path arrives from the frontend and
    // a relative one means nothing to the portal, which has its own cwd.
    let real = std::fs::canonicalize(&path).map_err(|e| format!("{path}: {e}"))?;
    let uri = arlen_launch_contract::file_uri(&real.to_string_lossy());
    tauri_plugin_arlen_portal::api::open_uri(
        &uri,
        tauri_plugin_arlen_portal::OpenUriOptions {
            ask: Some(true),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| e.to_string())
}
