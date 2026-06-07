//! Capability-based, fail-closed filesystem scope for the read-only File
//! Manager MCP server.
//!
//! The server reads only paths under a configured allowlist of canonical
//! directory roots. The default allowlist is **empty**: nothing is readable
//! until the user configures roots, so a fresh server exposes no filesystem at
//! all. Which roots to grant is the user's deliberate privacy choice, not a
//! default this server invents.
//!
//! Each root is held as an open cap-std [`Dir`] capability. Every read goes
//! through that capability with a path relative to it, so cap-std resolves the
//! path with `openat` internally and **refuses any symlink or `..` escape at
//! access time**. There is no canonicalise-then-reopen-by-name step, so the
//! check-then-reopen TOCTOU race does not exist: the open capability is the
//! authority, and the access either stays within it or fails.

use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use cap_std::ambient_authority;
use cap_std::fs::MetadataExt as _;
use cap_std::fs::{Dir, FileType, Metadata};

/// Why access was refused. Both are fail-closed denials.
#[derive(Debug, PartialEq, Eq)]
pub enum ScopeError {
    /// Not under any allowed root, contains `..`, or escapes its capability.
    NotPermitted,
    /// Under an allowed root but the path does not exist or cannot be read.
    Unresolvable,
}

/// One directory entry, reported without following symlinks.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct EntryInfo {
    pub name: String,
    pub kind: &'static str,
    pub size: Option<u64>,
    pub modified_unix: Option<u64>,
}

/// A directory listing, capped, with a truncation marker.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct DirListing {
    pub entries: Vec<EntryInfo>,
    pub truncated: bool,
}

/// Metadata for a single path.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct PathInfo {
    pub kind: &'static str,
    pub size: u64,
    pub modified_unix: Option<u64>,
    pub readonly: bool,
}

/// An allowed root: its canonical path (for matching) and the open capability.
struct Root {
    canonical: PathBuf,
    dir: Dir,
}

/// A fail-closed allowlist of canonical directory roots, each an open
/// capability. Empty means "deny everything".
pub struct Scope {
    roots: Vec<Root>,
}

impl Scope {
    /// Build a scope from configured root paths. Each root is canonicalised and
    /// opened as a capability; a root that does not resolve or cannot be opened
    /// is dropped (it could grant nothing). An empty result denies all.
    pub fn new<I, P>(roots: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let roots = roots
            .into_iter()
            .filter_map(|p| {
                let canonical = p.as_ref().canonicalize().ok()?;
                // Identity of the canonical dir before we open it.
                let expected = std::fs::metadata(&canonical).ok()?;
                let dir = Dir::open_ambient_dir(&canonical, ambient_authority()).ok()?;
                // Tie the opened capability to that identity: if the path was
                // swapped (e.g. a component replaced by a symlink) between the
                // canonicalise and the open, the opened dir's (dev, ino) differ
                // and we drop the root, rather than later translating
                // old-prefix requests onto a different tree.
                let opened = dir.dir_metadata().ok()?;
                if opened.ino() != expected.ino() || opened.dev() != expected.dev() {
                    return None;
                }
                Some(Root { canonical, dir })
            })
            .collect();
        Self { roots }
    }

    /// Whether the scope grants access to nothing (the fail-closed default).
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Find the capability and the path relative to it for a requested path.
    /// The requested path must be absolute and made only of normal components
    /// (no `.`/`..`), and lexically under a root. cap-std enforces, at access
    /// time, that the relative path does not escape the capability.
    fn resolve(&self, requested: &Path) -> Result<(&Dir, PathBuf), ScopeError> {
        if !requested.is_absolute() {
            return Err(ScopeError::NotPermitted);
        }
        for c in requested.components() {
            match c {
                Component::RootDir | Component::Normal(_) => {}
                // `..`, `.`, or a Windows prefix: refuse rather than normalise.
                _ => return Err(ScopeError::NotPermitted),
            }
        }
        for root in &self.roots {
            if let Ok(rel) = requested.strip_prefix(&root.canonical) {
                // An empty relative path means the root itself; cap-std reads
                // the directory it was opened on with ".".
                let rel = if rel.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    rel.to_path_buf()
                };
                return Ok((&root.dir, rel));
            }
        }
        Err(ScopeError::NotPermitted)
    }

    /// List the directory at `requested`, returning at most `cap` entries.
    /// Reads through the capability, so a symlink or `..` escape is refused.
    pub fn list(&self, requested: &Path, cap: usize) -> Result<DirListing, ScopeError> {
        let (dir, rel) = self.resolve(requested)?;
        let read_dir = dir.read_dir(&rel).map_err(|_| ScopeError::Unresolvable)?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for entry in read_dir {
            if entries.len() >= cap {
                truncated = true;
                break;
            }
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            // The entry's own type, never following a symlink to classify it.
            let ft = entry.file_type().ok();
            let (kind, size, modified) = match ft {
                // A symlink is reported as such with no target attributes, so a
                // link is never silently followed for size/time.
                Some(t) if t.is_symlink() => ("symlink", None, None),
                Some(t) => {
                    let meta = entry.metadata().ok();
                    (
                        kind_of(&t),
                        meta.as_ref().map(Metadata::len),
                        meta.as_ref().and_then(modified_unix),
                    )
                }
                None => ("other", None, None),
            };
            entries.push(EntryInfo {
                name,
                kind,
                size,
                modified_unix: modified,
            });
        }
        Ok(DirListing { entries, truncated })
    }

    /// Metadata for the path at `requested`. Reads through the capability, so a
    /// symlink or `..` escape is refused.
    pub fn stat(&self, requested: &Path) -> Result<PathInfo, ScopeError> {
        let (dir, rel) = self.resolve(requested)?;
        let meta = dir.metadata(&rel).map_err(|_| ScopeError::Unresolvable)?;
        Ok(PathInfo {
            kind: kind_of(&meta.file_type()),
            size: meta.len(),
            modified_unix: modified_unix(&meta),
            readonly: meta.permissions().readonly(),
        })
    }
}

/// Classify a file type into a stable wire string.
fn kind_of(ft: &FileType) -> &'static str {
    if ft.is_dir() {
        "dir"
    } else if ft.is_file() {
        "file"
    } else if ft.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

/// Seconds since the Unix epoch for a metadata mtime, or `None` if unavailable.
fn modified_unix(meta: &Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.into_std().duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_scope_denies_everything() {
        let scope = Scope::new(Vec::<PathBuf>::new());
        assert!(scope.is_empty());
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(scope.stat(tmp.path()), Err(ScopeError::NotPermitted));
        assert_eq!(scope.list(tmp.path(), 100), Err(ScopeError::NotPermitted));
    }

    #[test]
    fn an_allowed_root_lists_and_stats_its_children() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("sub")).unwrap();
        fs::write(root.path().join("f.txt"), b"hi").unwrap();
        let scope = Scope::new([root.path()]);

        let listing = scope.list(root.path(), 100).unwrap();
        assert!(!listing.truncated);
        assert_eq!(listing.entries.len(), 2, "sub + f.txt");
        let info = scope.stat(&root.path().join("f.txt")).unwrap();
        assert_eq!(info.kind, "file");
        assert_eq!(info.size, 2);
    }

    #[test]
    fn a_path_with_dotdot_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new([root.path()]);
        let escaping = root.path().join("..").join("etc");
        assert_eq!(scope.stat(&escaping), Err(ScopeError::NotPermitted));
    }

    #[test]
    fn a_symlink_escaping_the_root_is_refused_at_access() {
        // The capability refuses to follow a symlink out of the root, so the
        // out-of-scope target's metadata is never returned. This holds even
        // though the symlink lives inside the allowed root.
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"x").unwrap();
        let link = allowed.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path().join("secret"), &link).unwrap();

        let scope = Scope::new([allowed.path()]);
        // Listing the root shows the link as a symlink (not followed)...
        #[cfg(unix)]
        {
            let listing = scope.list(allowed.path(), 100).unwrap();
            assert!(listing.entries.iter().any(|e| e.name == "escape" && e.kind == "symlink"));
            // ...and statting through the link is refused, not followed out.
            assert_eq!(scope.stat(&link), Err(ScopeError::Unresolvable));
        }
    }

    #[test]
    fn a_sibling_prefix_is_not_permitted() {
        let base = tempfile::tempdir().unwrap();
        let foo = base.path().join("foo");
        let foobar = base.path().join("foobar");
        fs::create_dir(&foo).unwrap();
        fs::create_dir(&foobar).unwrap();
        let scope = Scope::new([&foo]);
        assert!(scope.stat(&foo).is_ok());
        assert_eq!(scope.stat(&foobar), Err(ScopeError::NotPermitted));
    }

    #[test]
    fn list_caps_entries_and_marks_truncation() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..3 {
            fs::write(root.path().join(format!("f{i}")), b"x").unwrap();
        }
        let capped = scope_for(&root).list(root.path(), 2).unwrap();
        assert_eq!(capped.entries.len(), 2);
        assert!(capped.truncated);
        let full = scope_for(&root).list(root.path(), 10).unwrap();
        assert_eq!(full.entries.len(), 3);
        assert!(!full.truncated);
    }

    fn scope_for(root: &tempfile::TempDir) -> Scope {
        Scope::new([root.path()])
    }

    #[test]
    fn an_unresolvable_root_is_dropped_not_granted() {
        let scope = Scope::new(["/this/root/does/not/exist"]);
        assert!(scope.is_empty());
    }
}
