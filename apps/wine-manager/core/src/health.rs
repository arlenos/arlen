//! Whether a bottle's prefix still matches what its description says.
//!
//! The window lists what `bottle.toml` records. The prefix is a directory a
//! person can open, and Wine writes to it every time the bottle runs: `winecfg`
//! adds a drive, a re-run of `wineboot -u` can put back the links the severing
//! cut, an installer maps its own letter. None of that changes the description,
//! so a window reading the description alone will keep saying the bottle reaches
//! two folders while the prefix says otherwise.
//!
//! This is the same rule as everything else here: a surface must not state
//! something nobody measured. So the description is a claim, and this is the
//! reading that either backs it or does not.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bottle::Bottle;
use crate::{dosdevices, map_drives, sever, Drive};

/// What a reading of the prefix found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Health {
    /// Letters the description expects that the prefix does not have. The program
    /// will not see the folder at all.
    pub missing: Vec<char>,
    /// Letters in the prefix that no grant asked for. Something else wrote them,
    /// and they reach wherever they point.
    pub unexpected: Vec<char>,
    /// Links that leave the prefix without a grant behind them: `Z:` put back, or
    /// a shell folder relinked into the home.
    pub escapes: Vec<PathBuf>,
}

impl Health {
    /// Whether the prefix says the same thing the description does.
    pub fn agrees(&self) -> bool {
        self.missing.is_empty() && self.unexpected.is_empty() && self.escapes.is_empty()
    }
}

/// Read a bottle's prefix and compare it with the bottle's own description.
///
/// A prefix that is not there at all is not a disagreement: the bottle was
/// recorded and never booted, which the caller can see from the empty reading and
/// the absent directory. An unreadable prefix surfaces as an error, because
/// "cannot tell" and "agrees" are the two answers that must never be confused.
pub fn check_bottle(bottle: &Bottle) -> std::io::Result<Health> {
    if !bottle.prefix_root.join("dosdevices").is_dir() {
        return Ok(Health::default());
    }
    let expected: Vec<Drive> = map_drives(&bottle.grants).unwrap_or_default();
    let have = dosdevices::granted_letters(&bottle.prefix_root)?;

    let mut health = Health {
        missing: expected
            .iter()
            .map(|d| d.letter)
            .filter(|l| !have.contains(l))
            .collect(),
        unexpected: have
            .iter()
            .copied()
            .filter(|l| !expected.iter().any(|d| d.letter == *l))
            .collect(),
        escapes: Vec::new(),
    };

    // The granted paths are the ones a link is allowed to leave for; anything else
    // reaching out of the prefix is something nobody asked for.
    let granted: Vec<PathBuf> = bottle.grants.iter().map(|g| g.host.clone()).collect();
    health.escapes = sever::still_escaping(&bottle.prefix_root, &granted)?;
    Ok(health)
}

/// Bring a prefix back to what its description says.
///
/// Three things, and each is one already-tested step: cut every link that leaves
/// the prefix without a grant behind it, write the drive table from the grants
/// again (which removes a letter nobody granted and restores one that went
/// missing), and read the result back so the caller is told what it is now rather
/// than what it was asked to be.
///
/// IT DOES NOT TOUCH `drive_c`. Everything a person or a program put inside the
/// bottle stays; what changes is the set of doors out of it and the names the
/// granted folders are reached by. A repair that could delete somebody's saved
/// file would be a worse answer than a bottle that disagrees with its record.
pub fn repair_bottle(bottle: &Bottle) -> std::io::Result<Health> {
    if !is_booted(&bottle.prefix_root) {
        return Ok(Health::default());
    }
    let granted: Vec<PathBuf> = bottle.grants.iter().map(|g| g.host.clone()).collect();
    let links = sever::prefix_links(&bottle.prefix_root)?;
    sever::apply(&sever::plan(&bottle.prefix_root, &links, &granted))?;

    if let Ok(drives) = map_drives(&bottle.grants) {
        // A grant list too long to map leaves the table alone rather than emptying
        // it: refusing to change anything is the honest answer to a description
        // that cannot be realised.
        dosdevices::write_drives(&bottle.prefix_root, &drives)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }
    check_bottle(bottle)
}

/// Whether `path` is a bottle prefix that has been booted.
pub fn is_booted(prefix_root: &Path) -> bool {
    prefix_root.join("dosdevices").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottle::Egress;
    use crate::{Access, PathGrant};

    /// A prefix under a name of its own, because these run in parallel threads of
    /// one process and a shared directory has them deleting each other's.
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("arlen-health-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bottle(tag: &str, grants: Vec<PathGrant>) -> (Bottle, PathBuf) {
        let dir = scratch(tag);
        let prefix = dir.join(tag);
        std::fs::create_dir_all(prefix.join("dosdevices")).unwrap();
        std::os::unix::fs::symlink("../drive_c", prefix.join("dosdevices/c:")).unwrap();
        let b = Bottle {
            id: tag.into(),
            prefix_root: prefix,
            grants,
            egress: Egress::None,
            plumbing: Default::default(),
        };
        (b, dir)
    }


    fn grant(host: &str) -> PathGrant {
        PathGrant { host: PathBuf::from(host), access: Access::ReadOnly }
    }

    #[test]
    fn a_prefix_written_from_the_description_agrees_with_it() {
        let (b, _d) = bottle("ok", vec![grant("/srv/a"), grant("/srv/b")]);
        dosdevices::write_drives(&b.prefix_root, &map_drives(&b.grants).unwrap()).unwrap();
        assert_eq!(check_bottle(&b).unwrap(), Health::default());
        assert!(check_bottle(&b).unwrap().agrees());
    }

    #[test]
    fn a_letter_nobody_granted_is_reported() {
        // What `winecfg` does when somebody adds a drive by hand, and what an
        // installer does when it maps one for itself.
        let (b, _d) = bottle("extra", vec![grant("/srv/a")]);
        dosdevices::write_drives(&b.prefix_root, &map_drives(&b.grants).unwrap()).unwrap();
        std::os::unix::fs::symlink("/etc", b.prefix_root.join("dosdevices/m:")).unwrap();
        let h = check_bottle(&b).unwrap();
        assert_eq!(h.unexpected, vec!['M']);
        assert!(!h.agrees());
    }

    #[test]
    fn a_granted_folder_with_no_letter_is_reported() {
        let (b, _d) = bottle("missing", vec![grant("/srv/a")]);
        let h = check_bottle(&b).unwrap();
        assert_eq!(h.missing, vec!['D'], "the program cannot see the folder at all");
    }

    #[test]
    fn the_filesystem_drive_put_back_is_an_escape() {
        // The one that matters: somebody ran `wineboot -u` in the prefix and Wine
        // wrote `z: -> /` again. The description still says two folders.
        let (b, _d) = bottle("zback", vec![grant("/srv/a")]);
        dosdevices::write_drives(&b.prefix_root, &map_drives(&b.grants).unwrap()).unwrap();
        std::os::unix::fs::symlink("/", b.prefix_root.join("dosdevices/z:")).unwrap();
        let h = check_bottle(&b).unwrap();
        assert!(!h.escapes.is_empty(), "{h:?}");
        assert!(h.escapes[0].ends_with("z:"));
    }

    #[test]
    fn a_repair_shuts_the_doors_and_leaves_the_contents_alone() {
        let (b, _d) = bottle("repair", vec![grant("/srv/a")]);
        // A drive nobody granted, the filesystem drive put back, and a file
        // somebody saved inside the bottle.
        std::os::unix::fs::symlink("/etc", b.prefix_root.join("dosdevices/m:")).unwrap();
        std::os::unix::fs::symlink("/", b.prefix_root.join("dosdevices/z:")).unwrap();
        let saved = b.prefix_root.join("drive_c/users/u/Documents");
        std::fs::create_dir_all(&saved).unwrap();
        std::fs::write(saved.join("letter.txt"), b"mine").unwrap();

        let after = repair_bottle(&b).unwrap();
        assert!(after.agrees(), "{after:?}");
        assert!(!b.prefix_root.join("dosdevices/z:").exists(), "the filesystem drive is gone");
        assert!(!b.prefix_root.join("dosdevices/m:").exists(), "and so is the letter nobody granted");
        assert_eq!(
            std::fs::read_link(b.prefix_root.join("dosdevices/d:")).unwrap(),
            PathBuf::from("/srv/a"),
            "while the granted folder has its letter",
        );
        assert_eq!(
            std::fs::read_to_string(saved.join("letter.txt")).unwrap(),
            "mine",
            "and nothing inside the bottle was touched",
        );
    }

    #[test]
    fn repairing_an_unbooted_bottle_does_nothing_rather_than_failing() {
        let dir = scratch("repair-unbooted");
        let b = Bottle {
            id: "x".into(),
            prefix_root: dir.join("pfx"),
            grants: vec![grant("/srv/a")],
            egress: Egress::None,
            plumbing: Default::default(),
        };
        assert_eq!(repair_bottle(&b).unwrap(), Health::default());
    }

    #[test]
    fn a_bottle_that_was_never_booted_is_not_a_disagreement() {
        let dir = scratch("unbooted");
        let b = Bottle {
            id: "unbooted".into(),
            prefix_root: dir.join("pfx"),
            grants: vec![grant("/srv/a")],
            egress: Egress::None,
            plumbing: Default::default(),
        };
        assert_eq!(check_bottle(&b).unwrap(), Health::default());
        assert!(!is_booted(&b.prefix_root));
    }
}
