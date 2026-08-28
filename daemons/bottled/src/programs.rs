//! Finding what an installer left behind, so a person can say which of it is the
//! app.
//!
//! WHY THIS IS A LIST AND NOT AN ANSWER. A Windows installer does not report what
//! it installed. It writes files, usually several programs' worth: the app, an
//! uninstaller, a crash reporter, a launcher, sometimes a bundled runtime. Picking
//! one automatically means guessing, and a bottle whose `Launch` starts the
//! uninstaller because it guessed wrong is worse than one that asked. So the
//! daemon answers with what it FOUND and the person says which it is.
//!
//! The obvious refinements are deliberately absent. Wine writes Start Menu
//! shortcuts, which would rank the candidates well, but they are `.lnk` files - a
//! binary shell-link format - and parsing one to save somebody a click is a parser
//! this daemon would then own. The compat recipe would answer it outright, and
//! `windows-apps-plan.md` has that as its own piece.
//!
//! The walk is bounded in three ways and each has a reason: it skips the Windows
//! directory (which is Wine's own several hundred stub programs, none of them
//! anybody's app), it never follows a symlink (a prefix is full of links pointing
//! at the person's home - see `crate::caches` for what happens when a walk
//! forgets), and it stops at a depth and a count, because an answer nobody can
//! read through is not an answer.

use std::path::{Path, PathBuf};

/// How deep under `drive_c` to look. Program Files, a vendor, a product and a
/// `bin` is four; twice that is generous without walking a source tree somebody
/// unpacked.
const MAX_DEPTH: usize = 8;

/// How many candidates to answer with. A person picks from a list they can read.
const MAX_RESULTS: usize = 200;

/// Directories under `drive_c` that hold no app of the person's.
///
/// `windows` is Wine's own implementation. `arlen-installers` is where we put the
/// installer, and offering it back as the program to run would make every bottle's
/// app its own setup program.
const SKIP_DIRS: &[&str] = &["windows", "arlen-installers"];

/// File names that are never the app, however they are cased. Inno Setup and
/// NSIS both leave one of these beside the program they installed.
const NOT_THE_APP: &[&str] = &["unins000.exe", "uninstall.exe", "uninstaller.exe"];

/// One program found in a prefix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    /// The host path, which is what a launch would run.
    pub path: PathBuf,
    /// The file name, which is what a person recognises.
    pub name: String,
}

/// Every program found under a bottle's C: drive, best-guess order.
///
/// Sorted by path so two calls agree; the caller decides how to present them. An
/// empty answer is a real one: an installer that was cancelled leaves nothing, and
/// saying so is better than an invented entry.
pub fn candidates(prefix_root: &Path) -> Vec<Candidate> {
    let mut found = Vec::new();
    walk(&prefix_root.join("drive_c"), 0, &mut found);
    found.sort();
    found.truncate(MAX_RESULTS);
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<Candidate>) {
    if depth > MAX_DEPTH || found.len() >= MAX_RESULTS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if found.len() >= MAX_RESULTS {
            return;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        // Never followed, in either direction: the links out of a prefix lead to
        // the person's real folders, and a program found through one is not in
        // this bottle at all.
        if meta.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.is_dir() {
            if depth == 0 && SKIP_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
                continue;
            }
            walk(&path, depth + 1, found);
        } else if meta.is_file() && is_program(&name) {
            found.push(Candidate { path, name });
        }
    }
}

/// Whether a file name is a program somebody might want to run.
fn is_program(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".exe") && !NOT_THE_APP.contains(&lower.as_str())
}

/// Whether a path may be recorded as a bottle's program.
///
/// INSIDE THE PREFIX, and this is the check rather than a formality. The program
/// is what `Launch` runs, so a caller that could record any host path would have
/// turned "start my Windows app" into "start whatever I name, under Wine, with
/// this bottle's grants". Compared after canonicalising both sides, so a `..` or a
/// symlink cannot spell a path that reads as inside and resolves as outside.
pub fn is_inside_prefix(prefix_root: &Path, program: &Path) -> bool {
    let (Ok(prefix), Ok(target)) = (prefix_root.canonicalize(), program.canonicalize()) else {
        // A path that does not resolve is not one to record: the file has to be
        // there for the check to mean anything.
        return false;
    };
    target.starts_with(&prefix) && target.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arlen-programs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"MZ").unwrap();
    }

    #[test]
    fn the_app_is_offered_and_wines_own_stubs_are_not() {
        let prefix = scratch("found");
        let c = prefix.join("drive_c");
        touch(&c.join("Program Files/Vendor/Game/game.exe"));
        touch(&c.join("Program Files/Vendor/Game/unins000.exe"));
        touch(&c.join("Program Files/Vendor/Game/readme.txt"));
        // Wine's own directory, several hundred of these in a real prefix.
        touch(&c.join("windows/system32/notepad.exe"));
        // And the installer we copied in, which is not the app either.
        touch(&c.join("arlen-installers/setup.exe"));

        let names: Vec<String> = candidates(&prefix).into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["game.exe".to_string()]);
    }

    #[test]
    fn a_link_out_of_the_prefix_offers_nothing() {
        let outside = scratch("outside-progs");
        touch(&outside.join("private.exe"));

        let prefix = scratch("linked");
        let c = prefix.join("drive_c");
        std::fs::create_dir_all(&c).unwrap();
        std::os::unix::fs::symlink(&outside, c.join("Documents")).unwrap();

        assert!(
            candidates(&prefix).is_empty(),
            "a program reached through a link out of the prefix is not in this bottle"
        );
    }

    #[test]
    fn a_program_outside_the_prefix_cannot_be_recorded() {
        let prefix = scratch("inside-check");
        let inside = prefix.join("drive_c/app/game.exe");
        touch(&inside);
        assert!(is_inside_prefix(&prefix, &inside));

        let outside = scratch("outside-check");
        let elsewhere = outside.join("game.exe");
        touch(&elsewhere);
        assert!(
            !is_inside_prefix(&prefix, &elsewhere),
            "recording a host path would turn a launch into 'run whatever I name'"
        );

        // Spelled as if it were inside, resolving outside.
        let traversal = prefix.join("drive_c/../../..").join(
            elsewhere
                .strip_prefix("/")
                .unwrap_or(&elsewhere),
        );
        assert!(!is_inside_prefix(&prefix, &traversal));
        assert!(
            !is_inside_prefix(&prefix, &prefix.join("drive_c/app")),
            "a directory is not a program"
        );
    }
}
