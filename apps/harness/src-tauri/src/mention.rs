//! `@`-mention file context for the conversation composer (ai-app.md §2.1,
//! "supplement the KG context with ad-hoc files").
//!
//! Two read-only commands back the composer's `@` popover: `list_files`
//! autocompletes a path the user is typing, and `read_mention_file` reads a
//! picked file's text (capped) so the frontend can prepend it to the prompt.
//!
//! This is the user acting on their own filesystem from their own app, the
//! same trust level as a file-open dialog: the listing base defaults to the
//! home directory but an absolute path the user types is honoured. Both
//! commands are advisory and fail soft (an unreadable directory lists nothing;
//! an unreadable file errors), and neither ever writes.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One autocomplete suggestion for the `@` popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSuggestion {
    /// Absolute path, ready to hand back to `read_mention_file`.
    pub path: String,
    /// The entry's own name (the last path component), for display.
    pub name: String,
    /// True for a directory, so the UI can offer to descend rather than attach.
    pub is_dir: bool,
}

/// A picked file's text, capped, for prepending to the prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionContent {
    /// The absolute path that was read.
    pub path: String,
    /// The entry's own name, for the attachment chip.
    pub name: String,
    /// The file's text, truncated to the cap. Read lossily, so non-UTF-8 bytes
    /// become replacement characters rather than failing the read.
    pub content: String,
    /// True when the file was larger than the cap and `content` is a prefix.
    pub truncated: bool,
}

/// Maximum suggestions returned per keystroke, so a huge directory cannot flood
/// the popover or the IPC channel.
const MAX_SUGGESTIONS: usize = 20;

/// Maximum bytes read from a mentioned file. A mention supplements the prompt,
/// it is not a bulk upload: a larger file is truncated to this prefix so one
/// `@` cannot blow past the model's context window on its own.
const MAX_MENTION_BYTES: usize = 64 * 1024;

/// Expand a leading `~` to the home directory. A bare `~` or `~/...` resolves
/// against `dirs::home_dir`; anything else is returned unchanged. Pure.
fn expand_tilde(input: &str, home: Option<&Path>) -> PathBuf {
    match (input.strip_prefix("~/"), input == "~", home) {
        (Some(rest), _, Some(h)) => h.join(rest),
        (_, true, Some(h)) => h.to_path_buf(),
        _ => PathBuf::from(input),
    }
}

/// Split the typed query into the directory to list and the name prefix to
/// match within it. With a trailing slash (or an empty query) the whole query
/// is the directory and the prefix is empty; otherwise the part after the last
/// slash is the prefix. `base` is the directory an unqualified query lists.
/// Pure, so the splitting logic is unit-tested without touching the disk.
fn split_query(query: &str, base: &Path, home: Option<&Path>) -> (PathBuf, String) {
    if query.is_empty() {
        return (base.to_path_buf(), String::new());
    }
    let expanded = expand_tilde(query, home);
    let expanded_str = expanded.to_string_lossy();
    // A trailing separator means "list this directory", no name prefix.
    if expanded_str.ends_with('/') {
        return (expanded, String::new());
    }
    match expanded.parent() {
        // A relative single segment (no parent dir) lists the base with that
        // segment as the prefix; an absolute path uses its real parent.
        Some(parent) if !parent.as_os_str().is_empty() => {
            let prefix = expanded
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (parent.to_path_buf(), prefix)
        }
        _ => (base.to_path_buf(), query.to_string()),
    }
}

/// Read up to `MAX_SUGGESTIONS` entries of `dir` whose name starts with
/// `prefix` (case-insensitive), directories first then alphabetical. Returns
/// empty on any read error, so the popover degrades quietly.
fn read_suggestions(dir: &Path, prefix: &str) -> Vec<FileSuggestion> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix_lower = prefix.to_lowercase();
    let mut out: Vec<FileSuggestion> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !prefix_lower.is_empty() && !name.to_lowercase().starts_with(&prefix_lower) {
                return None;
            }
            // Hidden files only show when the prefix explicitly asks for them.
            if name.starts_with('.') && !prefix.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(FileSuggestion {
                path: e.path().to_string_lossy().into_owned(),
                name,
                is_dir,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out.truncate(MAX_SUGGESTIONS);
    out
}

/// Autocomplete a path the user is typing after `@`. Lists the matching
/// directory entries (see [`split_query`]); never errors, returning an empty
/// list when there is nothing to show or the directory cannot be read.
#[tauri::command]
pub async fn list_files(query: String) -> Vec<FileSuggestion> {
    let home = dirs::home_dir();
    let base = home.clone().unwrap_or_else(|| PathBuf::from("/"));
    let (dir, prefix) = split_query(query.trim(), &base, home.as_deref());
    read_suggestions(&dir, &prefix)
}

/// Read a mentioned file's text for prompt injection. Errors (a returned
/// `Err` string the UI surfaces) when the path is not a regular file or cannot
/// be read; truncates to [`MAX_MENTION_BYTES`] otherwise. Read-only.
#[tauri::command]
pub async fn read_mention_file(path: String) -> Result<MentionContent, String> {
    let p = expand_tilde(path.trim(), dirs::home_dir().as_deref());
    let meta = std::fs::metadata(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", p.display()));
    }
    let bytes = std::fs::read(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    let truncated = bytes.len() > MAX_MENTION_BYTES;
    let slice = &bytes[..bytes.len().min(MAX_MENTION_BYTES)];
    let content = String::from_utf8_lossy(slice).into_owned();
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned());
    Ok(MentionContent {
        path: p.to_string_lossy().into_owned(),
        name,
        content,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_resolves_against_home() {
        let home = PathBuf::from("/home/u");
        assert_eq!(expand_tilde("~/Docs", Some(&home)), PathBuf::from("/home/u/Docs"));
        assert_eq!(expand_tilde("~", Some(&home)), home);
        assert_eq!(expand_tilde("/etc", Some(&home)), PathBuf::from("/etc"));
        // No home resolvable: a `~` path is left as-is rather than guessed.
        assert_eq!(expand_tilde("~/x", None), PathBuf::from("~/x"));
    }

    #[test]
    fn empty_query_lists_the_base_with_no_prefix() {
        let base = PathBuf::from("/home/u");
        assert_eq!(split_query("", &base, None), (base.clone(), String::new()));
    }

    #[test]
    fn bare_fragment_lists_base_with_that_prefix() {
        let base = PathBuf::from("/home/u");
        let (dir, prefix) = split_query("Doc", &base, None);
        assert_eq!(dir, base);
        assert_eq!(prefix, "Doc");
    }

    #[test]
    fn absolute_path_splits_into_parent_and_prefix() {
        let base = PathBuf::from("/home/u");
        let (dir, prefix) = split_query("/etc/host", &base, None);
        assert_eq!(dir, PathBuf::from("/etc"));
        assert_eq!(prefix, "host");
    }

    #[test]
    fn trailing_slash_lists_that_directory() {
        let base = PathBuf::from("/home/u");
        let (dir, prefix) = split_query("/etc/", &base, None);
        assert_eq!(dir, PathBuf::from("/etc/"));
        assert_eq!(prefix, "");
    }

    #[test]
    fn tilde_path_splits_against_home() {
        let base = PathBuf::from("/home/u");
        let home = PathBuf::from("/home/u");
        let (dir, prefix) = split_query("~/Doc", &base, Some(&home));
        assert_eq!(dir, home);
        assert_eq!(prefix, "Doc");
    }

    #[test]
    fn read_suggestions_filters_sorts_and_hides_dotfiles() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("alpha")).unwrap();
        std::fs::write(tmp.path().join("apple.txt"), b"x").unwrap();
        std::fs::write(tmp.path().join("banana.txt"), b"x").unwrap();
        std::fs::write(tmp.path().join(".hidden"), b"x").unwrap();

        let all = read_suggestions(tmp.path(), "");
        // Dotfile hidden, directory first, then files alpha.
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["alpha", "apple.txt", "banana.txt"]);
        assert!(all[0].is_dir);

        let a = read_suggestions(tmp.path(), "a");
        assert_eq!(a.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["alpha", "apple.txt"]);

        // An explicit dot prefix reveals dotfiles.
        let dot = read_suggestions(tmp.path(), ".");
        assert_eq!(dot.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), [".hidden"]);
    }

    #[test]
    fn read_suggestions_on_a_missing_dir_is_empty() {
        assert!(read_suggestions(Path::new("/no/such/dir/here"), "").is_empty());
    }

    #[tokio::test]
    async fn read_mention_file_reads_and_caps() {
        let tmp = tempfile::tempdir().unwrap();
        let small = tmp.path().join("small.txt");
        std::fs::write(&small, b"hello").unwrap();
        let got = read_mention_file(small.to_string_lossy().into_owned()).await.unwrap();
        assert_eq!(got.content, "hello");
        assert!(!got.truncated);
        assert_eq!(got.name, "small.txt");

        let big = tmp.path().join("big.txt");
        std::fs::write(&big, vec![b'a'; MAX_MENTION_BYTES + 10]).unwrap();
        let got = read_mention_file(big.to_string_lossy().into_owned()).await.unwrap();
        assert_eq!(got.content.len(), MAX_MENTION_BYTES);
        assert!(got.truncated);
    }

    #[tokio::test]
    async fn read_mention_file_rejects_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let err = read_mention_file(tmp.path().to_string_lossy().into_owned()).await;
        assert!(err.is_err());
    }
}
