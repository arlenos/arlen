//! Deterministic, path-contained application of recipe source patches.
//!
//! A recipe may declare `[[source]] patches = [...]` (forage-recipes.md section
//! 5a): unified-diff files, applied in order to the extracted source before the
//! build. This applies them with a pure-Rust engine (`diffy`), not by shelling
//! out, so the result is deterministic (exact context match, no fuzz) and needs
//! no host `patch` tool in the build root.
//!
//! Patching runs on the unconfined host build directory (before the sandbox),
//! so path safety is enforced here, not delegated: every target path is taken
//! from the diff's `+++` header, stripped one leading component (git `b/`,
//! `-p1`), and rejected if it is absolute, contains `..`, or would be reached
//! through a symlink out of the source tree. Patch and file sizes are capped so
//! a hostile diff cannot exhaust memory or disk.
//!
//! Two residuals are accepted, neither reachable by a hostile diff alone (a diff
//! cannot emit a symlink as content, and patching is single-threaded). First,
//! the symlink check in [`safe_target`] and the later write are not one atomic
//! operation, so a *concurrent external process* racing the build dir could
//! plant a symlink in the window; closing it fully needs
//! `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` relative to a source-root
//! dirfd. Second, only UTF-8 patches against UTF-8 source files are supported; a
//! binary patch fails closed rather than applying.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// The null path a unified diff uses for a created or deleted file.
const DEV_NULL: &str = "/dev/null";

/// Size caps for patch application.
#[derive(Debug, Clone)]
pub struct PatchLimits {
    /// Maximum size of a single patch file.
    pub max_patch_bytes: u64,
    /// Maximum size of a source file read or written while patching.
    pub max_file_bytes: u64,
}

impl Default for PatchLimits {
    fn default() -> Self {
        PatchLimits {
            max_patch_bytes: 16 * 1024 * 1024,
            max_file_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A failure applying a patch.
#[derive(Debug, Error)]
pub enum PatchError {
    /// A patch or source file could not be read or written.
    #[error("patch io at {path}: {source}")]
    Io {
        /// The path involved.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A patch path escaped the recipe directory.
    #[error("patch path {0} escapes the recipe directory")]
    PatchEscapes(String),
    /// A patch or source file exceeded its size cap.
    #[error("{path} exceeds the {limit}-byte cap")]
    TooLarge {
        /// The oversized path.
        path: PathBuf,
        /// The cap that was exceeded.
        limit: u64,
    },
    /// A patch file was not valid UTF-8 (unified diffs are text).
    #[error("patch {0} is not valid UTF-8")]
    NotUtf8(PathBuf),
    /// A diff section could not be parsed as a unified diff.
    #[error("malformed diff in {patch}: {reason}")]
    Malformed {
        /// The patch file the bad section came from.
        patch: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
    /// A diff targeted a path that escapes the source tree or a symlink.
    #[error("diff targets unsafe path {0}")]
    UnsafeTarget(String),
    /// A hunk did not apply cleanly against the current source.
    #[error("hunk did not apply cleanly to {0}")]
    DoesNotApply(String),
}

/// Apply each declared patch, in order, to the extracted source at
/// `source_root`. Patch paths are resolved relative to `recipe_dir` and must
/// stay within it.
pub fn apply_patches(
    source_root: &Path,
    recipe_dir: &Path,
    patches: &[PathBuf],
    limits: &PatchLimits,
) -> Result<(), PatchError> {
    for rel in patches {
        let patch_path = contained_join(recipe_dir, rel)
            .ok_or_else(|| PatchError::PatchEscapes(rel.display().to_string()))?;
        let text = read_capped(&patch_path, limits.max_patch_bytes)?;
        let text = String::from_utf8(text).map_err(|_| PatchError::NotUtf8(patch_path.clone()))?;
        apply_patch_text(source_root, &patch_path, &text, limits)?;
    }
    Ok(())
}

/// Apply every file-diff in one patch file's text to `source_root`.
fn apply_patch_text(
    source_root: &Path,
    patch_path: &Path,
    text: &str,
    limits: &PatchLimits,
) -> Result<(), PatchError> {
    for chunk in split_file_diffs(text) {
        apply_file_diff(source_root, patch_path, &chunk, limits)?;
    }
    Ok(())
}

/// Apply a single file's unified diff.
fn apply_file_diff(
    source_root: &Path,
    patch_path: &Path,
    chunk: &str,
    limits: &PatchLimits,
) -> Result<(), PatchError> {
    let mut lines = chunk.lines();
    let orig = header_path(lines.next().unwrap_or(""), "--- ");
    let modified = header_path(lines.next().unwrap_or(""), "+++ ");

    let patch = diffy::Patch::from_str(chunk).map_err(|e| PatchError::Malformed {
        patch: patch_path.to_path_buf(),
        reason: e.to_string(),
    })?;

    // A `+++ /dev/null` deletes the file named by `---`; otherwise the target is
    // the `+++` path. Either way the path is taken from the header, stripped one
    // component, and contained in the source tree.
    if modified.as_deref() == Some(DEV_NULL) {
        let rel = strip_one(orig.as_deref().ok_or_else(|| PatchError::Malformed {
            patch: patch_path.to_path_buf(),
            reason: "deletion diff without an original path".into(),
        })?)
        .ok_or_else(|| PatchError::UnsafeTarget("/dev/null".into()))?;
        let target = safe_target(source_root, &rel)?;
        if target.exists() {
            std::fs::remove_file(&target).map_err(|source| PatchError::Io {
                path: target.clone(),
                source,
            })?;
        }
        return Ok(());
    }

    let rel = strip_one(modified.as_deref().ok_or_else(|| PatchError::Malformed {
        patch: patch_path.to_path_buf(),
        reason: "diff without a target path".into(),
    })?)
    .ok_or_else(|| PatchError::UnsafeTarget("(empty)".into()))?;
    let target = safe_target(source_root, &rel)?;

    // A `--- /dev/null` creates a new file from an empty base; otherwise patch
    // the existing content.
    let base = if orig.as_deref() == Some(DEV_NULL) {
        String::new()
    } else {
        let bytes = read_capped(&target, limits.max_file_bytes)?;
        String::from_utf8(bytes).map_err(|_| PatchError::Malformed {
            patch: patch_path.to_path_buf(),
            reason: format!("target {} is not UTF-8 text", target.display()),
        })?
    };

    let patched = diffy::apply(&base, &patch)
        .map_err(|_| PatchError::DoesNotApply(target.display().to_string()))?;
    if patched.len() as u64 > limits.max_file_bytes {
        return Err(PatchError::TooLarge {
            path: target,
            limit: limits.max_file_bytes,
        });
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PatchError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&target, patched).map_err(|source| PatchError::Io {
        path: target,
        source,
    })
}

/// Split a unified diff into one chunk per file, each starting at its `--- `
/// header. Any inter-file preamble (`diff --git`, `index`, mode lines) is
/// dropped: `diffy` parses from the `---`/`+++` pair, and each chunk ends at the
/// last hunk-content line so trailing preamble never leaks into it.
fn split_file_diffs(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        // Find the next `--- ` / `+++ ` header pair.
        if lines[i].starts_with("--- ")
            && lines.get(i + 1).is_some_and(|l| l.starts_with("+++ "))
        {
            let mut chunk = vec![lines[i], lines[i + 1]];
            let mut j = i + 2;
            while j < lines.len() && is_hunk_line(lines[j]) {
                chunk.push(lines[j]);
                j += 1;
            }
            chunks.push(chunk.join("\n") + "\n");
            i = j;
        } else {
            i += 1;
        }
    }
    chunks
}

/// Whether a line belongs to a hunk body (the marker `@@`, a context, added,
/// removed, or the no-final-newline line). Anything else ends the file-diff.
fn is_hunk_line(line: &str) -> bool {
    matches!(line.as_bytes().first(), Some(b'@' | b' ' | b'+' | b'-' | b'\\'))
}

/// The path in a `--- ` / `+++ ` header line, trimming a trailing tab-timestamp.
fn header_path(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let path = rest.split('\t').next().unwrap_or(rest).trim_end();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Strip one leading path component (git `a/` / `b/`, i.e. `-p1`). `/dev/null`
/// and any path with no separator yield `None`.
fn strip_one(path: &str) -> Option<PathBuf> {
    if path == DEV_NULL {
        return None;
    }
    let (_, rest) = path.split_once('/')?;
    if rest.is_empty() {
        None
    } else {
        Some(PathBuf::from(rest))
    }
}

/// Join `rel` under `root`, returning `None` if `rel` is absolute or escapes via
/// `..`/root components.
fn contained_join(root: &Path, rel: &Path) -> Option<PathBuf> {
    if rel.is_absolute() {
        return None;
    }
    for c in rel.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return None,
        }
    }
    Some(root.join(rel))
}

/// Resolve a stripped diff target under `source_root`, rejecting escapes and any
/// existing symlink along the path (so a planted link cannot redirect a write
/// out of the tree).
fn safe_target(source_root: &Path, rel: &Path) -> Result<PathBuf, PatchError> {
    let joined =
        contained_join(source_root, rel).ok_or_else(|| PatchError::UnsafeTarget(rel.display().to_string()))?;
    // Walk each existing ancestor and the target itself; none may be a symlink.
    let mut probe = source_root.to_path_buf();
    for c in rel.components() {
        probe.push(c);
        if let Ok(meta) = std::fs::symlink_metadata(&probe) {
            if meta.file_type().is_symlink() {
                return Err(PatchError::UnsafeTarget(rel.display().to_string()));
            }
        }
    }
    Ok(joined)
}

/// Read a file, failing if it exceeds `cap` bytes.
fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, PatchError> {
    let meta = std::fs::metadata(path).map_err(|source| PatchError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if meta.len() > cap {
        return Err(PatchError::TooLarge {
            path: path.to_path_buf(),
            limit: cap,
        });
    }
    std::fs::read(path).map_err(|source| PatchError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    fn write_patch(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        PathBuf::from(name)
    }

    #[test]
    fn modifies_an_existing_file() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        write(src.path(), "hello.txt", "line one\nline two\n");
        let diff = "--- a/hello.txt\n+++ b/hello.txt\n@@ -1,2 +1,2 @@\n line one\n-line two\n+line TWO\n";
        let patches = vec![write_patch(recipe.path(), "fix.patch", diff)];
        apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(src.path().join("hello.txt")).unwrap(),
            "line one\nline TWO\n"
        );
    }

    #[test]
    fn creates_a_new_file() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        let diff = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,1 @@\n+fresh\n";
        let patches = vec![write_patch(recipe.path(), "add.patch", diff)];
        apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(src.path().join("new.txt")).unwrap(),
            "fresh\n"
        );
    }

    #[test]
    fn deletes_a_file() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        write(src.path(), "gone.txt", "bye\n");
        let diff = "--- a/gone.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\n";
        let patches = vec![write_patch(recipe.path(), "rm.patch", diff)];
        apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()).unwrap();
        assert!(!src.path().join("gone.txt").exists());
    }

    #[test]
    fn applies_a_multi_file_git_patch() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        write(src.path(), "a.txt", "aaa\n");
        write(src.path(), "b.txt", "bbb\n");
        let diff = "diff --git a/a.txt b/a.txt\nindex 111..222 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n-aaa\n+AAA\ndiff --git a/b.txt b/b.txt\nindex 333..444 100644\n--- a/b.txt\n+++ b/b.txt\n@@ -1,1 +1,1 @@\n-bbb\n+BBB\n";
        let patches = vec![write_patch(recipe.path(), "multi.patch", diff)];
        apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()).unwrap();
        assert_eq!(std::fs::read_to_string(src.path().join("a.txt")).unwrap(), "AAA\n");
        assert_eq!(std::fs::read_to_string(src.path().join("b.txt")).unwrap(), "BBB\n");
    }

    #[test]
    fn rejects_a_patch_path_escaping_the_recipe_dir() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        let patches = vec![PathBuf::from("../evil.patch")];
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::PatchEscapes(_)) => {}
            other => panic!("expected PatchEscapes, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_diff_target_escaping_the_source_tree() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        // After stripping `b/`, the target is `../../etc/passwd`: must be refused.
        let diff = "--- /dev/null\n+++ b/../../etc/passwd\n@@ -0,0 +1,1 @@\n+pwned\n";
        let patches = vec![write_patch(recipe.path(), "escape.patch", diff)];
        // Rejection before any write is the guarantee; the match arm proves it.
        // (A filesystem assertion here would resolve `..` to a real system path.)
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::UnsafeTarget(_)) => {}
            other => panic!("expected UnsafeTarget, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_write_through_a_symlink() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "original\n").unwrap();
        // A symlink inside the tree pointing outside it.
        std::os::unix::fs::symlink(&secret, src.path().join("link.txt")).unwrap();
        let diff = "--- a/link.txt\n+++ b/link.txt\n@@ -1,1 +1,1 @@\n-original\n+hacked\n";
        let patches = vec![write_patch(recipe.path(), "sym.patch", diff)];
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::UnsafeTarget(_)) => {}
            other => panic!("expected UnsafeTarget, got {other:?}"),
        }
        // The outside file is untouched.
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "original\n");
    }

    #[test]
    fn a_nonapplying_hunk_is_reported() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        write(src.path(), "f.txt", "totally different\n");
        let diff = "--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-expected\n+changed\n";
        let patches = vec![write_patch(recipe.path(), "bad.patch", diff)];
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::DoesNotApply(_)) => {}
            other => panic!("expected DoesNotApply, got {other:?}"),
        }
    }

    #[test]
    fn oversized_patch_is_rejected() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        let patches = vec![write_patch(recipe.path(), "big.patch", "--- a/x\n+++ b/x\n")];
        let limits = PatchLimits {
            max_patch_bytes: 4,
            max_file_bytes: 64,
        };
        match apply_patches(src.path(), recipe.path(), &patches, &limits) {
            Err(PatchError::TooLarge { .. }) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn strip_one_drops_the_leading_component() {
        assert_eq!(strip_one("a/file"), Some(PathBuf::from("file")));
        assert_eq!(strip_one("b/sub/file"), Some(PathBuf::from("sub/file")));
        assert_eq!(strip_one("/dev/null"), None);
        assert_eq!(strip_one("nosep"), None);
        assert_eq!(strip_one("a/"), None);
    }

    #[test]
    fn contained_join_rejects_escapes() {
        let root = Path::new("/root");
        assert_eq!(contained_join(root, Path::new("a/b")), Some(PathBuf::from("/root/a/b")));
        assert_eq!(contained_join(root, Path::new("a/./b")), Some(PathBuf::from("/root/a/./b")));
        assert_eq!(contained_join(root, Path::new("../x")), None);
        assert_eq!(contained_join(root, Path::new("a/../b")), None);
        assert_eq!(contained_join(root, Path::new("/abs")), None);
    }

    #[test]
    fn header_path_trims_tab_timestamp_and_prefix() {
        assert_eq!(
            header_path("--- a/hello.txt\t2024-01-01 12:00", "--- "),
            Some("a/hello.txt".to_string())
        );
        assert_eq!(header_path("+++ b/x.txt", "+++ "), Some("b/x.txt".to_string()));
        // Wrong prefix yields nothing.
        assert_eq!(header_path("--- a/x", "+++ "), None);
        // Empty/whitespace-only path yields nothing.
        assert_eq!(header_path("--- ", "--- "), None);
        assert_eq!(header_path("---  \t", "--- "), None);
    }

    #[test]
    fn is_hunk_line_classifies_body_vs_preamble() {
        for hunk in ["@@ -1 +1 @@", " context", "+added", "-removed", "\\ No newline"] {
            assert!(is_hunk_line(hunk), "`{hunk}` should be a hunk line");
        }
        for other in ["diff --git a/x b/x", "index 1..2 100644", ""] {
            assert!(!is_hunk_line(other), "`{other}` should not be a hunk line");
        }
    }

    #[test]
    fn split_file_diffs_separates_files_and_drops_preamble() {
        let text = "diff --git a/a.txt b/a.txt\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+A\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-b\n+B\n";
        let chunks = split_file_diffs(text);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with("--- a/a.txt\n+++ b/a.txt"));
        assert!(chunks[1].starts_with("--- a/b.txt\n+++ b/b.txt"));
        assert!(!chunks[0].contains("diff --git"));
        assert!(split_file_diffs("").is_empty());
    }

    #[test]
    fn rejects_a_non_utf8_patch_file() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        std::fs::write(recipe.path().join("bin.patch"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        let patches = vec![PathBuf::from("bin.patch")];
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::NotUtf8(_)) => {}
            other => panic!("expected NotUtf8, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_non_utf8_target_file() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("blob.bin"), [0xff, 0xfe, 0x00]).unwrap();
        let diff = "--- a/blob.bin\n+++ b/blob.bin\n@@ -1 +1 @@\n-x\n+y\n";
        let patches = vec![write_patch(recipe.path(), "p.patch", diff)];
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::Malformed { .. }) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_malformed_hunk() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        write(src.path(), "f.txt", "a\n");
        let diff = "--- a/f.txt\n+++ b/f.txt\n@@ this is not a hunk header @@\n+x\n";
        let patches = vec![write_patch(recipe.path(), "bad.patch", diff)];
        match apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()) {
            Err(PatchError::Malformed { .. }) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn rejects_patched_output_over_the_file_cap() {
        // Created-file path: base is empty, so the post-apply size check (not
        // read_capped) is what must reject the oversized result.
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        let diff = "--- /dev/null\n+++ b/big.txt\n@@ -0,0 +1,1 @@\n+0123456789\n";
        let patches = vec![write_patch(recipe.path(), "add.patch", diff)];
        let limits = PatchLimits {
            max_patch_bytes: 1 << 20,
            max_file_bytes: 4,
        };
        match apply_patches(src.path(), recipe.path(), &patches, &limits) {
            Err(PatchError::TooLarge { .. }) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
        assert!(!src.path().join("big.txt").exists());
    }

    #[test]
    fn applies_patches_in_declared_order() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        write(src.path(), "v.txt", "one\n");
        let first = write_patch(
            recipe.path(),
            "1.patch",
            "--- a/v.txt\n+++ b/v.txt\n@@ -1 +1 @@\n-one\n+two\n",
        );
        let second = write_patch(
            recipe.path(),
            "2.patch",
            "--- a/v.txt\n+++ b/v.txt\n@@ -1 +1 @@\n-two\n+three\n",
        );
        apply_patches(src.path(), recipe.path(), &[first, second], &PatchLimits::default()).unwrap();
        assert_eq!(std::fs::read_to_string(src.path().join("v.txt")).unwrap(), "three\n");
    }

    #[test]
    fn creates_a_file_in_a_new_subdirectory() {
        let src = tempfile::tempdir().unwrap();
        let recipe = tempfile::tempdir().unwrap();
        let diff = "--- /dev/null\n+++ b/sub/dir/n.txt\n@@ -0,0 +1,1 @@\n+nested\n";
        let patches = vec![write_patch(recipe.path(), "nest.patch", diff)];
        apply_patches(src.path(), recipe.path(), &patches, &PatchLimits::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(src.path().join("sub/dir/n.txt")).unwrap(),
            "nested\n"
        );
    }
}
