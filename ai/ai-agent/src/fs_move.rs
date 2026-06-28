//! Collision-safe move planning for the executor's `fs.move` arm (the
//! tidy-downloads go-live, the first live executor action).
//!
//! A move MUST stay reversible: it never overwrites an occupied destination
//! (design-doc gap F4 - an overwrite destroys the occupant, an irreversible
//! act). On a name collision the plan picks a free sibling name (`name (1).ext`,
//! `name (2).ext`, ...), so the file always lands somewhere new and the inverse
//! is a clean [`InverseReceipt::RestorePath`] from where it landed back to its
//! source.
//!
//! Pure: the occupancy predicate is injected, so the planning + the inverse are
//! unit-tested without a filesystem. The executor arm supplies the real
//! `Path::exists`; this module decides WHERE the file may safely land and what
//! undoes it.

use crate::effect_model::{CanonicalPath, InverseReceipt};

/// The bounded number of collision-avoiding suffixes tried before giving up.
/// Fail-closed: rather than overwrite or loop unbounded, a destination dir with
/// 1000 same-named files yields no plan.
const MAX_SUFFIX: u32 = 1000;

/// A planned collision-safe move: where the file will land (never an occupied
/// path) and the inverse that restores it to its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePlan {
    /// The source path (canonical, absolute).
    pub source: CanonicalPath,
    /// The resolved destination: the requested name, or a free `name (n).ext`
    /// sibling when the requested name was occupied. Never an occupied path.
    pub destination: CanonicalPath,
    /// The undo - move back from `destination` to `source`.
    pub inverse: InverseReceipt,
    /// True when the destination was renamed to dodge a collision.
    pub renamed: bool,
}

/// Plan a collision-safe move of `source` into directory `dest_dir`.
/// `occupied(path)` reports whether a path already exists (the executor injects
/// `Path::exists`; tests inject a set).
///
/// Returns `None` - fail-closed, never overwrite or guess - when the source is
/// not a canonical absolute path, has no file-name component, `dest_dir` is not
/// a canonical absolute directory, the only free candidate would be the source
/// itself (a no-op the caller must not dress up as a move), or no free name is
/// found within [`MAX_SUFFIX`].
pub fn plan_move(
    source: &str,
    dest_dir: &str,
    occupied: impl Fn(&str) -> bool,
) -> Option<MovePlan> {
    let source = CanonicalPath::new(source)?;
    let dir = CanonicalPath::new(dest_dir)?;
    let dir = trim_trailing_slash(dir.as_str());
    let name = std::path::Path::new(source.as_str())
        .file_name()
        .and_then(|n| n.to_str())?;
    let (stem, ext) = split_name(name);

    for n in 0..=MAX_SUFFIX {
        let candidate = if n == 0 {
            format!("{dir}/{name}")
        } else {
            format!("{dir}/{stem} ({n}){ext}")
        };
        // Moving a file onto itself is a no-op, never a "successful move" - and
        // counting the source as occupied would silently overwrite it. Refuse.
        if candidate == source.as_str() {
            return None;
        }
        if occupied(&candidate) {
            continue;
        }
        let destination = CanonicalPath::new(&candidate)?;
        let inverse = InverseReceipt::RestorePath {
            now: destination.clone(),
            prior: source.clone(),
        };
        return Some(MovePlan {
            source,
            destination,
            inverse,
            renamed: n != 0,
        });
    }
    None
}

/// Split a file name into `(stem, extension-with-dot)`. A leading dot (dotfile)
/// or no dot yields an empty extension, so the collision suffix lands before any
/// real extension: `report.pdf` -> `report (1).pdf`; `.bashrc` -> `.bashrc (1)`;
/// `archive.tar.gz` -> `archive.tar (1).gz` (the last dot, matching the OS).
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        // index 0 is a dotfile, not an extension separator.
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Strip one trailing `/` so `dir.join(name)` does not double it. A canonical
/// path never has a trailing slash except a bare root, which `CanonicalPath`
/// already rejects, so this is belt-and-braces.
fn trim_trailing_slash(s: &str) -> &str {
    s.strip_suffix('/').unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "/home/u/Downloads/report.pdf";
    const DIR: &str = "/home/u/Documents/Projects";

    #[test]
    fn no_collision_keeps_the_requested_name() {
        let plan = plan_move(SRC, DIR, |_| false).expect("plan");
        assert_eq!(plan.destination.as_str(), "/home/u/Documents/Projects/report.pdf");
        assert!(!plan.renamed);
        assert_eq!(
            plan.inverse,
            InverseReceipt::RestorePath {
                now: plan.destination.clone(),
                prior: plan.source.clone(),
            }
        );
    }

    #[test]
    fn a_collision_renames_before_the_extension_never_overwrites() {
        let taken = "/home/u/Documents/Projects/report.pdf";
        let plan = plan_move(SRC, DIR, |p| p == taken).expect("plan");
        assert_eq!(plan.destination.as_str(), "/home/u/Documents/Projects/report (1).pdf");
        assert!(plan.renamed);
        assert_ne!(plan.destination.as_str(), taken, "never the occupied path");
    }

    #[test]
    fn a_collision_chain_walks_to_the_first_free_suffix() {
        let occupied = |p: &str| {
            p == "/home/u/Documents/Projects/report.pdf"
                || p == "/home/u/Documents/Projects/report (1).pdf"
        };
        let plan = plan_move(SRC, DIR, occupied).expect("plan");
        assert_eq!(plan.destination.as_str(), "/home/u/Documents/Projects/report (2).pdf");
    }

    #[test]
    fn a_dotfile_suffixes_after_the_whole_name() {
        let src = "/home/u/Downloads/.bashrc";
        let taken = "/home/u/Documents/Projects/.bashrc";
        let plan = plan_move(src, DIR, |p| p == taken).expect("plan");
        assert_eq!(plan.destination.as_str(), "/home/u/Documents/Projects/.bashrc (1)");
    }

    #[test]
    fn a_double_extension_suffixes_at_the_last_dot() {
        let src = "/home/u/Downloads/archive.tar.gz";
        let taken = "/home/u/Documents/Projects/archive.tar.gz";
        let plan = plan_move(src, DIR, |p| p == taken).expect("plan");
        assert_eq!(plan.destination.as_str(), "/home/u/Documents/Projects/archive.tar (1).gz");
    }

    #[test]
    fn a_relative_source_or_dir_is_refused() {
        assert!(plan_move("Downloads/report.pdf", DIR, |_| false).is_none());
        assert!(plan_move(SRC, "relative/dir", |_| false).is_none());
        assert!(plan_move(SRC, "/", |_| false).is_none());
    }

    #[test]
    fn moving_a_file_onto_itself_is_refused_not_overwritten() {
        // Source already in the destination dir, nothing occupied: the n=0
        // candidate equals the source, so refuse rather than no-op-overwrite.
        let plan = plan_move("/home/u/Documents/Projects/report.pdf", DIR, |_| false);
        assert!(plan.is_none());
    }
}
