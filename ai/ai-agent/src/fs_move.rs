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

/// The filesystem move primitive the executor's `fs.move` arm calls. A seam so
/// the arm is unit-tested with an in-memory mover; [`OsFileMover`] is the real
/// on-disk impl. The planner guarantees `to` is a free path, so an
/// implementation never has to (and must never) overwrite.
pub trait FileMover: Send + Sync {
    /// Move `from` to `to`. `to` is a planner-chosen free path (never occupied).
    fn move_file(&self, from: &str, to: &str) -> std::io::Result<()>;
}

/// `EXDEV` (cross-device link) on Linux: `rename(2)` fails with it when source
/// and destination are on different mounts, so the mover falls back to
/// copy-then-remove. A named const since this crate has no `libc` dep.
const EXDEV: i32 = 18;

/// The real on-disk mover: `rename`, falling back to copy+remove across
/// filesystems (a `~/Downloads` -> `~/Documents/Projects` move can cross a
/// mount). Never overwrites.
///
/// The planner chose a destination free at plan time, but `std::fs::rename`
/// (and `std::fs::copy`) silently overwrite an existing target, so a file that
/// appeared at that exact path between plan and move would be destroyed (the
/// plan->move TOCTOU). To keep "never overwrite" true at the syscall, the move
/// first ATOMICALLY claims the destination with `create_new` (`O_EXCL`): if the
/// path now exists, the open fails `AlreadyExists` and the move is refused; if
/// it succeeds we own an empty placeholder, and the rename/copy below only ever
/// replaces our own claim. (`renameat2`+`RENAME_NOREPLACE` would be the kernel
/// primitive, but it needs an unsafe `libc` call this `forbid(unsafe_code)`
/// crate cannot make; the claim is the safe-std equivalent.)
pub struct OsFileMover;

impl FileMover for OsFileMover {
    fn move_file(&self, from: &str, to: &str) -> std::io::Result<()> {
        // Atomically claim the destination, refusing if it now exists. This
        // closes the plan->move race: after this open succeeds, `to` is ours, so
        // the rename/copy can only replace our empty placeholder, never a real
        // file (an `AlreadyExists` here is the non-overwrite refusal).
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        match std::fs::rename(from, to) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(EXDEV) => {
                // Cross-device: copy over our placeholder (we own it), then drop
                // the source. On a copy failure, roll back the placeholder.
                if let Err(copy_err) = std::fs::copy(from, to) {
                    let _ = std::fs::remove_file(to);
                    return Err(copy_err);
                }
                std::fs::remove_file(from)
            }
            Err(e) => {
                // The rename failed; remove the placeholder we created so a retry
                // (or a later move) does not see our empty claim as an occupant.
                let _ = std::fs::remove_file(to);
                Err(e)
            }
        }
    }
}

/// Execute a planned collision-safe move via `mover` and return the inverse that
/// undoes it. The plan (slice 1) already chose a free, non-overwriting
/// destination, so this just performs the move; the inverse is the plan's
/// `RestorePath`. The caller audits BEFORE invoking this (audit-before-act) and
/// records the returned inverse for the undo/compensate path.
pub fn execute_move<'a>(
    plan: &'a MovePlan,
    mover: &dyn FileMover,
) -> std::io::Result<&'a InverseReceipt> {
    mover.move_file(plan.source.as_str(), plan.destination.as_str())?;
    Ok(&plan.inverse)
}

#[cfg(test)]
mod execute_tests {
    use super::*;
    use std::io::Write;

    /// Plan then execute a real move on disk, then apply the returned inverse:
    /// the file lands at the (free) destination and the inverse restores it to
    /// the source - the reversibility the gate lifts on, proven end to end.
    #[test]
    fn a_planned_move_executes_and_its_inverse_restores() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let projects = dir.path().join("Projects");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        let src = downloads.join("report.pdf");
        std::fs::File::create(&src).unwrap().write_all(b"hi").unwrap();

        let plan = plan_move(
            src.to_str().unwrap(),
            projects.to_str().unwrap(),
            |p| std::path::Path::new(p).exists(),
        )
        .expect("plan");
        let dst = projects.join("report.pdf");
        assert_eq!(plan.destination.as_str(), dst.to_str().unwrap());

        let inverse = execute_move(&plan, &OsFileMover).expect("execute").clone();
        assert!(dst.exists(), "moved to the destination");
        assert!(!src.exists(), "gone from the source");

        // Apply the inverse (the undo): move back.
        match &inverse {
            InverseReceipt::RestorePath { now, prior } => {
                OsFileMover.move_file(now.as_str(), prior.as_str()).expect("restore");
            }
            other => panic!("expected RestorePath, got {other:?}"),
        }
        assert!(src.exists(), "restored to the source");
        assert!(!dst.exists(), "no longer at the destination");
    }

    /// On a collision the executed move lands at the renamed sibling, never
    /// clobbering the occupant.
    #[test]
    fn a_collision_move_lands_at_the_renamed_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let projects = dir.path().join("Projects");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        let src = downloads.join("report.pdf");
        std::fs::File::create(&src).unwrap().write_all(b"new").unwrap();
        let occupant = projects.join("report.pdf");
        std::fs::File::create(&occupant).unwrap().write_all(b"old").unwrap();

        let plan = plan_move(
            src.to_str().unwrap(),
            projects.to_str().unwrap(),
            |p| std::path::Path::new(p).exists(),
        )
        .expect("plan");
        execute_move(&plan, &OsFileMover).expect("execute");

        assert_eq!(std::fs::read_to_string(&occupant).unwrap(), "old", "occupant untouched");
        assert_eq!(
            std::fs::read_to_string(projects.join("report (1).pdf")).unwrap(),
            "new",
            "moved file landed at the renamed sibling"
        );
    }

    /// If a file appears at the planned destination AFTER the plan but BEFORE the
    /// move (the plan->move TOCTOU), `OsFileMover` refuses rather than overwrite:
    /// the atomic `create_new` claim fails `AlreadyExists` and the occupant is
    /// untouched, the source intact.
    #[test]
    fn the_mover_refuses_to_overwrite_a_destination_that_appeared_after_planning() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        std::fs::File::create(&src).unwrap().write_all(b"src").unwrap();
        // Plan chose `dst` while it was free; now a racer plants a file there.
        std::fs::File::create(&dst).unwrap().write_all(b"racer").unwrap();

        let err = OsFileMover
            .move_file(src.to_str().unwrap(), dst.to_str().unwrap())
            .expect_err("must refuse an occupied destination");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "racer", "occupant untouched");
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "src", "source intact");
    }
}
