//! Clickable file references - the chat file-pill backend seam (surface 1 of
//! `harness-file-refs-and-diffs-plan.md`).
//!
//! Two commands back a file pill the agent names in its prose or a tool card:
//!
//! - [`file_ref_resolve`] tells the pill how to render: the basename, whether
//!   the path resolves (an unresolvable path renders muted, "not found", with no
//!   click target - the honest state the IDEs paper over), and the MIME type
//!   from the SAME resolution that decides which app opens it (so the icon
//!   previews what clicking does).
//! - [`open_file_as_user`] opens the file with the USER's authority through the
//!   desktop default-MIME-opener (`xdg-open`, which routes through
//!   `xdg-desktop-portal` on a portal-enabled desktop) - the powerbox principle:
//!   the authority to open comes from the user's click, NOT the agent's
//!   capability-scoped read slice, so this NEVER touches the AI daemon and never
//!   appears in the agent-read ledger. The path is passed as a single argument
//!   (no shell), so it carries no injection risk.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// How a file pill should render, returned by [`file_ref_resolve`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileRef {
    /// The resolved absolute path when the file exists, else the input verbatim.
    pub path: String,
    /// The basename for the pill label.
    pub name: String,
    /// Whether the path resolves to an existing filesystem object. A broken
    /// symlink, a deleted file, or a never-existed path reads as `false` (the
    /// pill renders muted with no click target).
    pub resolvable: bool,
    /// Whether the resolved object is a directory (clicking opens the file
    /// manager rather than a document viewer).
    pub is_dir: bool,
    /// The MIME type from `xdg-mime query filetype`, or `None` when the path is
    /// unresolvable or no MIME database answers. The frontend maps it to an icon.
    pub mime: Option<String>,
}

/// The basename of `path` for the pill label. Falls back to the whole input when
/// the path has no final component (e.g. `/`). Pure, so it is unit-tested.
fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Query the system MIME database for `path`'s type via `xdg-mime`. Returns
/// `None` when `xdg-mime` is absent, fails, or answers empty. This is the same
/// resolution that selects the opener, so the pill icon and the open action
/// agree.
fn query_mime(path: &str) -> Option<String> {
    let output = Command::new("xdg-mime")
        .args(["query", "filetype", path])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mime = String::from_utf8(output.stdout).ok()?;
    let trimmed = mime.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Resolve a file pill: basename, resolvable state, is-dir and MIME. The path is
/// canonicalised so the pill carries the real absolute path (following symlinks
/// to their target); an unresolvable path keeps the input verbatim and reports
/// `resolvable: false`.
#[tauri::command]
pub async fn file_ref_resolve(path: String) -> FileRef {
    match std::fs::canonicalize(&path) {
        Ok(real) => {
            let real_str = real.to_string_lossy().into_owned();
            let is_dir = real.is_dir();
            // MIME only for files; a directory's opener is the file manager.
            let mime = if is_dir { None } else { query_mime(&real_str) };
            FileRef {
                name: basename(&real_str),
                path: real_str,
                resolvable: true,
                is_dir,
                mime,
            }
        }
        Err(_) => FileRef {
            name: basename(&path),
            path,
            resolvable: false,
            is_dir: false,
            mime: None,
        },
    }
}

/// Open `path` with the user's default handler via `xdg-open`. Refuses a path
/// that does not resolve (a muted pill has no click target, so this is the
/// fail-safe backstop). Opening is the user's own authority exercised through
/// the desktop opener - it does NOT go through the AI daemon or the agent's
/// capability-scoped reads.
#[tauri::command]
pub async fn open_file_as_user(path: String) -> Result<(), String> {
    let real = std::fs::canonicalize(&path).map_err(|_| format!("not found: {path}"))?;
    Command::new("xdg-open")
        .arg(&real)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("xdg-open: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_takes_the_final_component() {
        assert_eq!(basename("/home/tim/notes.md"), "notes.md");
        assert_eq!(basename("relative/path/file.rs"), "file.rs");
        assert_eq!(basename("bare.txt"), "bare.txt");
        // No final component falls back to the whole input.
        assert_eq!(basename("/"), "/");
    }

    #[tokio::test]
    async fn an_existing_file_resolves_with_an_absolute_path_and_basename() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, b"hi").unwrap();

        let r = file_ref_resolve(file.to_string_lossy().into_owned()).await;
        assert!(r.resolvable);
        assert!(!r.is_dir);
        assert_eq!(r.name, "hello.txt");
        assert!(r.path.ends_with("hello.txt"));
        assert!(Path::new(&r.path).is_absolute());
    }

    #[tokio::test]
    async fn an_existing_directory_resolves_as_a_dir_with_no_mime() {
        let dir = tempfile::tempdir().unwrap();
        let r = file_ref_resolve(dir.path().to_string_lossy().into_owned()).await;
        assert!(r.resolvable);
        assert!(r.is_dir);
        assert_eq!(r.mime, None);
    }

    #[tokio::test]
    async fn a_missing_path_is_unresolvable_and_keeps_the_input() {
        let r = file_ref_resolve("/no/such/path/ghost.md".to_string()).await;
        assert!(!r.resolvable);
        assert!(!r.is_dir);
        assert_eq!(r.mime, None);
        assert_eq!(r.name, "ghost.md");
        assert_eq!(r.path, "/no/such/path/ghost.md");
    }

    #[tokio::test]
    async fn a_broken_symlink_reads_as_unresolvable() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("gone"), &link).unwrap();
        let r = file_ref_resolve(link.to_string_lossy().into_owned()).await;
        assert!(!r.resolvable);
    }

    #[tokio::test]
    async fn opening_a_missing_path_is_refused() {
        let err = open_file_as_user("/no/such/path/ghost.md".to_string())
            .await
            .unwrap_err();
        assert!(err.contains("not found"));
    }
}
