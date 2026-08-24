//! Cutting a fresh prefix loose from the home it was wired into.
//!
//! `wineboot` writes the eight links the crate documentation lists, so severing
//! them is a step that runs once per bottle, after Wine has created the prefix and
//! before the program is ever started in it. Doing it earlier does not work: Wine
//! writes them itself on first boot, so a prefix that was cut before booting is a
//! prefix that has not booted yet.
//!
//! Two of the three kinds are treated differently, and the difference is the whole
//! module:
//!
//! `dosdevices/z:` is deleted and nothing replaces it. A program that finds no
//! `Z:` treats it as a drive that is not mounted, which is a state Windows
//! programs have always had to handle.
//!
//! The shell folders under `drive_c/users/<user>` are deleted and a real directory
//! is created in their place. Deleting them alone would leave `Documents` missing,
//! and a program that cannot find My Documents does not degrade politely, it
//! fails to save. The replacement is what `winecfg`'s Desktop Integration tab
//! does when a folder is unlinked, so the resulting prefix is one Wine itself
//! would produce.
//!
//! The `com1..com32` links to `/dev/ttyS*` are left alone. They point at device
//! nodes, and under the bottle's `--dev` there is no `/dev/ttyS0` for them to
//! reach, so they dangle rather than grant anything. Removing thirty-two links to
//! change nothing would only make the prefix differ from a stock one for no gain,
//! and a bottle that is later granted a serial port wants them back.

use std::path::{Path, PathBuf};

use crate::{escapes, Reach};

/// What to do with one link that leaves the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sever {
    /// Delete the link and leave nothing behind (`dosdevices/z:`).
    Remove(PathBuf),
    /// Delete the link and create a real directory in its place (the shell
    /// folders, which programs expect to exist).
    Replace(PathBuf),
}

impl Sever {
    /// The link this severing acts on.
    pub fn path(&self) -> &Path {
        match self {
            Sever::Remove(p) | Sever::Replace(p) => p,
        }
    }
}

/// Decide what to do with each link, without touching anything.
///
/// Split from [`apply`] so the decision can be shown to someone before it is
/// made: creating a bottle deletes things inside a directory the user may have
/// been using as a plain prefix, and that is worth being able to preview.
///
/// `granted` is the same list [`still_escaping`] takes, for the same reason.
pub fn plan(prefix_root: &Path, links: &[(PathBuf, PathBuf)], granted: &[PathBuf]) -> Vec<Sever> {
    escapes(prefix_root, links)
        .into_iter()
        // A granted drive points out of the prefix because someone asked it to.
        // Without this, running the pass a second time on a bottle that already
        // has drives would delete the drive table and leave empty directories
        // where the letters were.
        .filter(|e| !granted.iter().any(|g| e.target.starts_with(g)))
        .filter_map(|e| match e.reach {
            Reach::Filesystem => Some(Sever::Remove(e.link)),
            // A drive letter and a shell folder are cut differently, and the
            // difference is what the program expects to find afterwards. An
            // ungranted letter must simply not be there: putting an empty
            // directory in its place would give the program a drive that mounts
            // nothing, which is worse than a drive that is absent. A shell folder
            // has to exist, because a program that cannot find My Documents fails
            // to save. Found by a bottle whose creation then failed at the drive
            // table, which tried to remove a letter that had become a directory.
            Reach::Host(_) if e.link.parent().is_some_and(|d| d.ends_with("dosdevices")) => {
                Some(Sever::Remove(e.link))
            }
            Reach::Host(_) => Some(Sever::Replace(e.link)),
            // Left alone deliberately; see the module documentation.
            Reach::Device(_) => None,
            Reach::Contained => None,
        })
        .collect()
}

/// Read every symlink under `prefix_root`, following none of them.
///
/// Bounded to the two directories Wine puts them in rather than walking the whole
/// prefix: a bottle's `drive_c` holds whatever the installed program wrote, and a
/// program's own symlinks are its business. `Program Files` full of relative links
/// is not something a boundary pass should be rewriting.
pub fn prefix_links(prefix_root: &Path) -> std::io::Result<Vec<(PathBuf, PathBuf)>> {
    let mut found = Vec::new();
    let mut roots = vec![prefix_root.join("dosdevices")];
    let users = prefix_root.join("drive_c/users");
    if users.is_dir() {
        for entry in std::fs::read_dir(&users)? {
            roots.push(entry?.path());
        }
    }
    for dir in roots {
        let listing = match std::fs::read_dir(&dir) {
            Ok(l) => l,
            // A prefix that has not booted has no dosdevices yet, and that is a
            // caller ordering mistake rather than an error to swallow silently,
            // so it surfaces as an empty plan and the caller's own check.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in listing {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                let target = std::fs::read_link(entry.path())?;
                found.push((entry.path(), target));
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Carry out a plan.
///
/// Returns what it did, so a caller can record the severing against the bottle
/// rather than assume it. An already-severed prefix is not an error: a bottle may
/// be re-checked at any time, and a second pass finds nothing to do.
pub fn apply(plan: &[Sever]) -> std::io::Result<Vec<PathBuf>> {
    let mut done = Vec::new();
    for step in plan {
        let path = step.path();
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(path)?,
            // Not a symlink any more: either already severed, or never was one.
            // Either way there is nothing here that reaches out of the prefix.
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
        if let Sever::Replace(p) = step {
            std::fs::create_dir_all(p)?;
        }
        done.push(path.to_path_buf());
    }
    Ok(done)
}

/// Every link that still leaves the prefix without having been granted.
///
/// `granted` is what the drive table was written from. Once drives exist, most of
/// what leaves the prefix leaves it on purpose, and a check that could only be run
/// before the bottle was usable is a check that would never be run again. So this
/// answers the question that stays interesting: what reaches out of here that
/// nobody asked for.
///
/// Devices are excluded for the reason given at the top of the module.
pub fn still_escaping(prefix_root: &Path, granted: &[PathBuf]) -> std::io::Result<Vec<PathBuf>> {
    Ok(escapes(prefix_root, &prefix_links(prefix_root)?)
        .into_iter()
        .filter(|e| !matches!(e.reach, Reach::Device(_)))
        .filter(|e| !granted.iter().any(|g| e.target.starts_with(g)))
        .map(|e| e.link)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filesystem_drive_is_removed_and_the_shell_folders_are_replaced() {
        let root = PathBuf::from("/p");
        let links = vec![
            (root.join("dosdevices/c:"), root.join("drive_c")),
            (root.join("dosdevices/z:"), PathBuf::from("/")),
            (root.join("dosdevices/com1"), PathBuf::from("/dev/ttyS0")),
            (
                root.join("drive_c/users/u/Documents"),
                PathBuf::from("/home/u/Documents"),
            ),
        ];
        assert_eq!(
            plan(&root, &links, &[]),
            vec![
                Sever::Remove(root.join("dosdevices/z:")),
                Sever::Replace(root.join("drive_c/users/u/Documents")),
            ]
        );
    }

    #[test]
    fn a_real_prefix_is_severed_and_stays_severed() {
        let tmp = std::env::temp_dir().join(format!("arlen-sever-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dos = tmp.join("dosdevices");
        let user = tmp.join("drive_c/users/u");
        std::fs::create_dir_all(&dos).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(tmp.join("drive_c")).unwrap();
        std::os::unix::fs::symlink("../drive_c", dos.join("c:")).unwrap();
        std::os::unix::fs::symlink("/", dos.join("z:")).unwrap();
        std::os::unix::fs::symlink("/dev/ttyS0", dos.join("com1")).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), user.join("Documents")).unwrap();

        let steps = plan(&tmp, &prefix_links(&tmp).unwrap(), &[]);
        assert_eq!(apply(&steps).unwrap().len(), 2);
        assert_eq!(still_escaping(&tmp, &[]).unwrap(), Vec::<PathBuf>::new());

        // The shell folder is now a real directory a program can save into, not a
        // hole where My Documents used to be.
        assert!(user.join("Documents").is_dir());
        assert!(!user
            .join("Documents")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        // The system drive and the serial links are untouched.
        assert!(dos
            .join("c:")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(dos
            .join("com1")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());

        // A second pass finds nothing to do and does not fail.
        assert_eq!(
            apply(&plan(&tmp, &prefix_links(&tmp).unwrap(), &[])).unwrap(),
            Vec::<PathBuf>::new()
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn an_ungranted_drive_letter_is_removed_and_not_replaced_by_a_directory() {
        let root = PathBuf::from("/p");
        let links = vec![
            (root.join("dosdevices/y:"), PathBuf::from("/etc")),
            (
                root.join("drive_c/users/u/Documents"),
                PathBuf::from("/home/u/Documents"),
            ),
        ];
        assert_eq!(
            plan(&root, &links, &[]),
            vec![
                Sever::Remove(root.join("dosdevices/y:")),
                Sever::Replace(root.join("drive_c/users/u/Documents")),
            ]
        );
    }

    #[test]
    fn a_second_pass_does_not_eat_the_drive_table() {
        // A granted drive is a symlink out of the prefix, so a repair pass that
        // did not know about grants would replace every drive letter with an
        // empty directory and call it isolation.
        let root = PathBuf::from("/p");
        let links = vec![(root.join("dosdevices/d:"), PathBuf::from("/srv/share"))];
        assert_eq!(plan(&root, &links, &[PathBuf::from("/srv/share")]), vec![]);
        assert_eq!(
            plan(&root, &links, &[]),
            vec![Sever::Remove(root.join("dosdevices/d:"))],
            "an ungranted letter goes away rather than becoming an empty directory"
        );
    }

    #[test]
    fn a_drive_pointing_at_a_granted_directory_is_not_an_escape() {
        let tmp = std::env::temp_dir().join(format!("arlen-granted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("dosdevices")).unwrap();
        std::os::unix::fs::symlink("/srv/share", tmp.join("dosdevices/d:")).unwrap();
        std::os::unix::fs::symlink("/srv/other", tmp.join("dosdevices/e:")).unwrap();
        assert_eq!(
            still_escaping(&tmp, &[PathBuf::from("/srv/share")]).unwrap(),
            vec![tmp.join("dosdevices/e:")],
            "the granted one is expected; the one nobody asked for is the finding"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_prefix_that_has_not_booted_yields_an_empty_plan_rather_than_an_error() {
        let tmp = std::env::temp_dir().join(format!("arlen-unbooted-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(prefix_links(&tmp).unwrap(), vec![]);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
