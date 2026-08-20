//! Making a bottle, from an id and a grant list to a prefix that is ready to run.
//!
//! The order of the steps is the design, not an implementation detail:
//!
//! 1. Boot the prefix. Wine writes the registry and, with it, the eight symlinks
//!    out of the prefix. There is no way to prevent that; a prefix cut before it
//!    boots is a prefix that has not booted.
//! 2. Cut those links. Everything the person did not ask for goes now, while the
//!    drive table is still the stock one and there is nothing of theirs to lose.
//! 3. Write the drive table. Only now, because a severing pass run after this
//!    would see each granted drive as a link out of the prefix. [`crate::sever`]
//!    takes the granted paths for exactly that reason, and this order means the
//!    first pass never has to.
//! 4. Save the description, last, so a bottle that exists on disk is one that was
//!    finished. A crash before this leaves a directory with no `bottle.toml`,
//!    which [`crate::registry::list_bottles`] deliberately neither lists nor
//!    reports.
//!
//! Booting is injected rather than called, so the sequence can be tested on a
//! machine with no Wine, and so the caller decides whether that boot is confined.

use std::path::{Path, PathBuf};

use crate::bottle::{Bottle, Egress};
use crate::plumbing::Plumbing;
use crate::registry::{self, RegistryError};
use crate::{dosdevices, sever, PathGrant};

/// What to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBottle {
    /// The bottle name, which is also its directory.
    pub id: String,
    /// What it may reach on the filesystem.
    pub grants: Vec<PathGrant>,
    /// What it may reach on the network.
    pub egress: Egress,
    /// What it needs beyond that.
    pub plumbing: Plumbing,
}

/// Why a bottle could not be made.
#[derive(Debug)]
pub enum CreateError {
    /// Something is already there under that name. Refused rather than merged: a
    /// bottle is a prefix with software installed in it, and overwriting one
    /// because the name matched would throw that away.
    AlreadyExists(PathBuf),
    /// Wine could not create the prefix.
    Boot(String),
    /// The prefix booted and something in it still reaches out of it. Reported
    /// rather than ignored, since it means Wine grew a link this code has never
    /// seen and the isolation claim would be false.
    StillEscapes(Vec<PathBuf>),
    /// The registry refused the id, or the write failed.
    Registry(RegistryError),
    /// The drive table could not be written.
    Drives(String),
    /// The filesystem said no.
    Io(std::io::Error),
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateError::AlreadyExists(p) => {
                write!(f, "there is already a bottle at {}", p.display())
            }
            CreateError::Boot(e) => write!(f, "the prefix could not be created: {e}"),
            CreateError::StillEscapes(l) => write!(
                f,
                "{} link(s) still reach out of the prefix after cutting, so it is not isolated",
                l.len()
            ),
            CreateError::Registry(e) => write!(f, "{e}"),
            CreateError::Drives(e) => write!(f, "{e}"),
            CreateError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CreateError {}

impl From<std::io::Error> for CreateError {
    fn from(e: std::io::Error) -> Self {
        CreateError::Io(e)
    }
}

impl From<RegistryError> for CreateError {
    fn from(e: RegistryError) -> Self {
        CreateError::Registry(e)
    }
}

/// Make a bottle under `bottles_dir`, booting the prefix with `boot`.
///
/// `boot` is handed the prefix path and is expected to run `wineboot` against it,
/// however the caller wants that done.
pub fn create_bottle(
    bottles_dir: &Path,
    new: &NewBottle,
    boot: impl Fn(&Path) -> Result<(), String>,
) -> Result<Bottle, CreateError> {
    let description = registry::bottle_path(bottles_dir, &new.id)?;
    if description.exists() {
        return Err(CreateError::AlreadyExists(description));
    }
    let prefix = registry::prefix_for(bottles_dir, &new.id)?;
    std::fs::create_dir_all(&prefix)?;

    boot(&prefix).map_err(CreateError::Boot)?;

    let links = sever::prefix_links(&prefix)?;
    sever::apply(&sever::plan(&prefix, &links, &[]))?;
    let left = sever::still_escaping(&prefix, &[])?;
    if !left.is_empty() {
        return Err(CreateError::StillEscapes(left));
    }

    let drives = crate::map_drives(&new.grants).map_err(|e| CreateError::Drives(e.to_string()))?;
    dosdevices::write_drives(&prefix, &drives).map_err(|e| CreateError::Drives(e.to_string()))?;

    let bottle = Bottle {
        id: new.id.clone(),
        prefix_root: prefix,
        grants: new.grants.clone(),
        egress: new.egress.clone(),
        plumbing: new.plumbing.clone(),
    };
    registry::save_bottle(bottles_dir, &bottle)?;
    Ok(bottle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Access;

    /// Stand in for `wineboot`: writes the links a real one writes, so the cutting
    /// step has something to cut.
    fn fake_boot(prefix: &Path) -> Result<(), String> {
        let dos = prefix.join("dosdevices");
        let user = prefix.join("drive_c/users/u");
        std::fs::create_dir_all(&dos).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&user).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(prefix.join("drive_c")).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink("../drive_c", dos.join("c:")).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink("/", dos.join("z:")).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink("/home/u/Documents", user.join("Documents"))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("arlen-create-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn new_bottle(id: &str) -> NewBottle {
        NewBottle {
            id: id.into(),
            grants: vec![PathGrant {
                host: PathBuf::from("/srv/share"),
                access: Access::ReadWrite,
            }],
            egress: Egress::None,
            plumbing: Plumbing::default(),
        }
    }

    #[test]
    fn a_made_bottle_is_cut_lettered_and_written_down() {
        let d = dir("made");
        let b = create_bottle(&d, &new_bottle("probe"), fake_boot).unwrap();
        assert!(!b.prefix_root.join("dosdevices/z:").exists(), "the filesystem drive is gone");
        assert!(
            b.prefix_root.join("drive_c/users/u/Documents").is_dir(),
            "and My Documents is a real local directory"
        );
        assert_eq!(
            std::fs::read_link(b.prefix_root.join("dosdevices/d:")).unwrap(),
            PathBuf::from("/srv/share")
        );
        assert_eq!(registry::load_bottle(&d, "probe").unwrap(), b);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn making_a_bottle_over_an_existing_one_is_refused() {
        let d = dir("exists");
        create_bottle(&d, &new_bottle("probe"), fake_boot).unwrap();
        assert!(matches!(
            create_bottle(&d, &new_bottle("probe"), fake_boot),
            Err(CreateError::AlreadyExists(_))
        ));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_ungranted_drive_letter_does_not_survive_creation() {
        // If Wine ever grows a link this code does not know how to cut, the bottle
        // must not be recorded as one: a description on disk is a claim that the
        // prefix is isolated.
        let d = dir("escapes");
        let boot = |prefix: &Path| -> Result<(), String> {
            fake_boot(prefix)?;
            std::os::unix::fs::symlink("/etc", prefix.join("dosdevices/y:"))
                .map_err(|e| e.to_string())
        };
        // An ungranted letter is removed outright rather than replaced by a
        // directory. The first version of this test failed here: the cutter left a
        // directory named `y:`, and writing the drive table then tried to remove a
        // letter that was not a file.
        let made = create_bottle(&d, &new_bottle("probe"), boot);
        assert!(made.is_ok(), "an ungranted letter is cut like the others");
        assert!(!d.join("probe/pfx/dosdevices/y:").exists(), "and nothing is left in its place");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_boot_that_fails_leaves_no_bottle_behind() {
        let d = dir("bootfail");
        let made = create_bottle(&d, &new_bottle("probe"), |_| Err("no wine here".into()));
        assert!(matches!(made, Err(CreateError::Boot(_))));
        assert!(registry::load_bottle(&d, "probe").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
