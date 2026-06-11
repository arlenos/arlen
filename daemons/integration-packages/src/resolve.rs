//! Source-glob resolution + the `instance_strategy` (integration-packages-plan.md IP-R2).
//!
//! A source path may be a glob (`~/.mozilla/firefox/*/prefs.js`) that matches
//! several files (a multi-profile app). [`glob_under`] resolves it to the concrete
//! matches, walking under a cap-std `Dir` capability rooted at the allowlist
//! directory: this is the ACCESS-TIME confinement that complements the
//! declared-path allowlist gate, so a symlink under the root that points outside
//! cannot widen the match (cap-std refuses the escaping `open_dir` at the
//! syscall). [`resolve`] then applies the [`InstanceStrategy`] to pick which match
//! to write.
//!
//! Only a whole-segment `*` is supported (`firefox/*/prefs.js`), which covers the
//! multi-profile / per-install config case the adapters target; a partial-segment
//! or recursive glob is not (the manifest author uses the whole-segment form).

use crate::adapter::InstanceStrategy;
use crate::allowlist::{resolve_under_allowlist, AllowlistError, ALLOWED_SUBDIRS};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// A failure resolving or globbing a confined source path.
#[derive(Debug, thiserror::Error)]
pub enum GlobError {
    /// The source path failed the user-config allowlist gate.
    #[error(transparent)]
    Allowlist(#[from] AllowlistError),
    /// The allowlist root directory could not be opened as a capability.
    #[error("opening allowlist root {root}: {error}")]
    OpenRoot {
        /// The root that could not be opened.
        root: String,
        /// The underlying I/O error.
        error: String,
    },
}

/// Open the cap-std capability rooted at the allowlist directory a validated
/// source path lives under, and return it with the root-RELATIVE glob.
///
/// This OWNS the root construction so the access-time confinement cannot be
/// miswired: a caller passes only `home` and the untrusted `raw_source_path`, and
/// gets back a `Dir` rooted at exactly the allowlist directory the path validated
/// against (e.g. `~/.mozilla`) plus the glob relative to it (e.g.
/// `firefox/*/prefs.js`). Globbing and any later I/O go through that `Dir`, so
/// they are confined to the allowlist subtree by construction; the caller cannot
/// accidentally root the capability at `/` or `$HOME`.
pub fn confined_root(raw_source_path: &str, home: &Path) -> Result<(Dir, String), GlobError> {
    let abs = resolve_under_allowlist(raw_source_path, home)?;
    // The allowlist guarantees `abs` is under `home/<sub>` for exactly one sub.
    let (root, relative) = ALLOWED_SUBDIRS
        .iter()
        .find_map(|sub| {
            let root = home.join(sub);
            abs.strip_prefix(&root)
                .ok()
                .map(|rel| (root, rel.to_string_lossy().into_owned()))
        })
        .expect("an allowlist-validated path is under exactly one allowlist root");
    let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|e| GlobError::OpenRoot {
        root: root.display().to_string(),
        error: e.to_string(),
    })?;
    Ok((dir, relative))
}

/// Resolve `raw_source_path` against the allowlist under `home`, open its
/// allowlist root, and glob it - the confined end-to-end entry point. The
/// matches' `rel_path` is relative to the allowlist root (re-derive it with
/// [`confined_root`] if you need the root capability for the subsequent I/O).
pub fn glob_confined(raw_source_path: &str, home: &Path) -> Result<Vec<Match>, GlobError> {
    let (dir, relative) = confined_root(raw_source_path, home)?;
    Ok(glob_under(&dir, &relative))
}

/// One file a source glob matched, with its modification time (for `last_used`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The match path, RELATIVE to the cap-std root it was found under.
    pub rel_path: String,
    /// Seconds since the Unix epoch of the file's mtime (0 if unavailable).
    pub modified_secs: u64,
}

/// Which file(s) an edit applies to, after the strategy is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Write exactly this file (`last_used`, or the sole match).
    One(String),
    /// Write every match (`all`).
    All(Vec<String>),
    /// Ambiguous: the user must pick one of these (`ask` with more than one match).
    Ask(Vec<String>),
    /// The glob matched no file.
    NoMatch,
}

/// Resolve a `relative_glob` (relative to the cap-std `root`) to its matches.
///
/// Supports a single whole-segment `*`: every segment before it is descended
/// literally, the `*` segment expands to each child name, and the remaining
/// segments are checked under each. A glob with no `*` resolves to the single
/// literal path if it exists. Every descent is a cap-std `open_dir`, so the walk
/// cannot escape `root` (an escaping symlink is refused, that branch dropped). An
/// unreadable directory or a missing leaf is simply not a match (never fatal).
///
/// PRECONDITION: `root` MUST be a `Dir` opened at the allowlist directory the
/// source path validated against (under `$HOME`), and `relative_glob` MUST be
/// root-relative (the allowlist prefix stripped), never the absolute path. Use
/// [`confined_root`] / [`glob_confined`], which own that construction, rather than
/// rooting the capability by hand.
pub fn glob_under(root: &Dir, relative_glob: &str) -> Vec<Match> {
    let segments: Vec<&str> = relative_glob.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Vec::new();
    }
    let star = segments.iter().position(|s| *s == "*");
    match star {
        None => {
            // No wildcard: the literal path is a match iff it is a readable file.
            if is_file(root, relative_glob) {
                vec![match_for(root, relative_glob.to_string())]
            } else {
                Vec::new()
            }
        }
        Some(idx) => {
            let before = segments[..idx].join("/");
            let after = segments[idx + 1..].join("/");
            // Descend to the directory holding the `*`, then expand each child.
            let Ok(parent) = open_subdir(root, &before) else {
                return Vec::new();
            };
            let Ok(entries) = parent.read_dir(".") else {
                return Vec::new();
            };
            let mut matches = Vec::new();
            for entry in entries.flatten() {
                let child = entry.file_name().to_string_lossy().into_owned();
                // Build the candidate path relative to the ORIGINAL root.
                let mut candidate = before.clone();
                if !candidate.is_empty() {
                    candidate.push('/');
                }
                candidate.push_str(&child);
                if !after.is_empty() {
                    candidate.push('/');
                    candidate.push_str(&after);
                }
                if is_file(root, &candidate) {
                    matches.push(match_for(root, candidate));
                }
            }
            matches.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
            matches
        }
    }
}

/// Apply the strategy to the matches.
pub fn resolve(strategy: InstanceStrategy, matches: &[Match]) -> Resolution {
    if matches.is_empty() {
        return Resolution::NoMatch;
    }
    match strategy {
        InstanceStrategy::All => {
            Resolution::All(matches.iter().map(|m| m.rel_path.clone()).collect())
        }
        InstanceStrategy::LastUsed => {
            // The most recently modified; ties break on the (sorted) path so the
            // pick is deterministic.
            let pick = matches
                .iter()
                .max_by(|a, b| {
                    a.modified_secs
                        .cmp(&b.modified_secs)
                        .then_with(|| b.rel_path.cmp(&a.rel_path))
                })
                .expect("matches is non-empty");
            Resolution::One(pick.rel_path.clone())
        }
        InstanceStrategy::Ask => {
            if matches.len() == 1 {
                Resolution::One(matches[0].rel_path.clone())
            } else {
                Resolution::Ask(matches.iter().map(|m| m.rel_path.clone()).collect())
            }
        }
    }
}

/// Open `rel` as a directory under `root`, descending segment by segment so the
/// cap-std confinement applies at every step. An empty path is the root itself.
fn open_subdir(root: &Dir, rel: &str) -> std::io::Result<Dir> {
    if rel.is_empty() {
        return root.open_dir(".");
    }
    root.open_dir(rel)
}

/// Whether `rel` resolves to a readable regular file under `root`. `cap-std`'s
/// `metadata` follows symlinks, but the follow is sandboxed to the capability, so
/// a symlink that points outside `root` is refused rather than followed out.
fn is_file(root: &Dir, rel: &str) -> bool {
    root.metadata(rel).map(|m| m.is_file()).unwrap_or(false)
}

/// Build a [`Match`] for `rel` under `root`, reading its mtime (0 if unavailable).
fn match_for(root: &Dir, rel: String) -> Match {
    let modified_secs = root
        .metadata(&rel)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            let st: SystemTime = t.into_std();
            st.duration_since(UNIX_EPOCH).ok()
        })
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Match {
        rel_path: rel,
        modified_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::fs;
    use std::path::Path as StdPath;

    fn cap(path: &StdPath) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    #[test]
    fn globs_a_star_segment_to_each_match() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("firefox/aaa")).unwrap();
        fs::create_dir_all(tmp.path().join("firefox/bbb")).unwrap();
        fs::write(tmp.path().join("firefox/aaa/prefs.js"), b"a").unwrap();
        fs::write(tmp.path().join("firefox/bbb/prefs.js"), b"b").unwrap();
        // A profile dir with no prefs.js is not a match.
        fs::create_dir_all(tmp.path().join("firefox/empty")).unwrap();

        let root = cap(tmp.path());
        let m = glob_under(&root, "firefox/*/prefs.js");
        let paths: Vec<&str> = m.iter().map(|x| x.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["firefox/aaa/prefs.js", "firefox/bbb/prefs.js"]);
    }

    #[test]
    fn a_literal_path_matches_only_when_the_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("app")).unwrap();
        fs::write(tmp.path().join("app/config.toml"), b"x").unwrap();
        let root = cap(tmp.path());
        assert_eq!(glob_under(&root, "app/config.toml").len(), 1);
        assert!(glob_under(&root, "app/missing.toml").is_empty());
        // A directory is not a file match.
        assert!(glob_under(&root, "app").is_empty());
    }

    fn m(path: &str, secs: u64) -> Match {
        Match {
            rel_path: path.to_string(),
            modified_secs: secs,
        }
    }

    #[test]
    fn resolve_applies_each_strategy() {
        let matches = [m("a/prefs.js", 100), m("b/prefs.js", 200)];
        // last_used -> the newer.
        assert_eq!(
            resolve(InstanceStrategy::LastUsed, &matches),
            Resolution::One("b/prefs.js".into())
        );
        // all -> both.
        assert_eq!(
            resolve(InstanceStrategy::All, &matches),
            Resolution::All(vec!["a/prefs.js".into(), "b/prefs.js".into()])
        );
        // ask with >1 -> ask.
        assert_eq!(
            resolve(InstanceStrategy::Ask, &matches),
            Resolution::Ask(vec!["a/prefs.js".into(), "b/prefs.js".into()])
        );
        // ask with exactly one -> no prompt needed.
        assert_eq!(
            resolve(InstanceStrategy::Ask, &matches[..1]),
            Resolution::One("a/prefs.js".into())
        );
        // no matches -> NoMatch for every strategy.
        assert_eq!(resolve(InstanceStrategy::LastUsed, &[]), Resolution::NoMatch);
    }

    #[test]
    fn last_used_breaks_ties_deterministically() {
        // Equal mtimes: the pick must be stable, not dependent on input order.
        let a = [m("a", 50), m("b", 50)];
        let b = [m("b", 50), m("a", 50)];
        assert_eq!(resolve(InstanceStrategy::LastUsed, &a), resolve(InstanceStrategy::LastUsed, &b));
    }

    #[test]
    fn glob_confined_roots_at_the_allowlist_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // A literal source under ~/.config, globbed through the owned root.
        fs::create_dir_all(home.join(".config/app")).unwrap();
        fs::write(home.join(".config/app/config.toml"), b"x").unwrap();
        let m = glob_confined("~/.config/app/config.toml", home).unwrap();
        assert_eq!(m.len(), 1);
        // The rel_path is relative to the .config allowlist root, not the home.
        assert_eq!(m[0].rel_path, "app/config.toml");

        // A *-glob under ~/.mozilla resolves to its match, root-relative.
        fs::create_dir_all(home.join(".mozilla/firefox/p1")).unwrap();
        fs::write(home.join(".mozilla/firefox/p1/prefs.js"), b"a").unwrap();
        let g = glob_confined("~/.mozilla/firefox/*/prefs.js", home).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].rel_path, "firefox/p1/prefs.js");

        // A source outside the allowlist is an allowlist error, not an empty glob.
        assert!(matches!(
            glob_confined("/etc/passwd", home),
            Err(GlobError::Allowlist(_))
        ));
    }
}
