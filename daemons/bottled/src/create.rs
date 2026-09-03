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

/// The `bwrap` argument list that boots a fresh prefix.
///
/// A prefix has to be booted before it is a prefix at all - Wine writes the
/// registry and the drive table on first run - and that boot is the one thing
/// that happens before there is a bottle to describe it. So it borrows the
/// launcher's assembly with a bottle that exists only for this call: the prefix as
/// its own home, NO grants and NO network. A boot has nothing of the person's to
/// read and nothing to fetch, and the first thing a new prefix should learn is
/// that it cannot reach either.
///
/// `wineboot -u` rather than plain `wine`: it is the documented way to ask Wine to
/// create or update a prefix and exit, instead of starting a program that happens
/// to have the side effect.
pub fn boot_argv(
    prefix_root: &Path,
    usr: &Path,
    runtime_dir: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Result<Vec<String>, crate::launch::LaunchError> {
    let scaffold = Bottle {
        // Never written to disk: `create_bottle` saves the real description after
        // the boot, and this one exists only to carry the prefix through the
        // launcher's assembly.
        id: "boot".to_string(),
        prefix_root: prefix_root.to_path_buf(),
        grants: Vec::new(),
        egress: Egress::None,
        plumbing: Plumbing::default(),
        program: Vec::new(),
    };
    crate::launch::launch_argv(
        &scaffold,
        usr,
        runtime_dir,
        // No display: `launch_env` already disables the Mono and Gecko modals,
        // which are the only thing a boot would want to draw.
        None,
        &["wineboot".to_string(), "-u".to_string()],
        exists,
    )
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
///
/// WHAT A FAILED CREATE LEAVES, because it is a real property and not a bug to
/// go looking for. Everything after `create_dir_all` can fail with a booted
/// prefix already on disk, and the description is saved last on purpose, so what
/// remains is a directory with no `bottle.toml`. That is invisible by design:
/// `registry::list_bottles` neither lists nor reports it, so no surface offers to
/// remove it either. Retrying the same id reuses it and finishes - `wineboot -u`
/// and the severing pass are both idempotent - which is the path back. Nobody
/// retrying means a prefix's worth of disk that only a file manager can reclaim,
/// and that is the trade for never showing a half-made bottle as a real one.
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
        // A fresh bottle runs nothing: the prefix is booted and empty until an
        // installer has put something in it.
        program: Vec::new(),
    };
    registry::save_bottle(bottles_dir, &bottle)?;
    Ok(bottle)
}

#[cfg(test)]
mod tests {
    /// Every path exists, so the assembly is not shaped by this host.
    fn all(_: &Path) -> bool {
        true
    }

    #[test]
    fn a_boot_reaches_nothing_of_the_persons_and_runs_wineboot() {
        let argv = boot_argv(
            Path::new("/var/lib/arlen/bottles/game/pfx"),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            all,
        )
        .expect("a host with wine assembles a boot");

        let sep = argv.iter().position(|a| a == "--").expect("a separator");
        assert_eq!(
            &argv[sep + 1..],
            &[
                crate::launch::WINE.to_string(),
                "wineboot".to_string(),
                "-u".to_string()
            ],
            "the boot asks Wine to make the prefix and exit, rather than starting a program"
        );

        // No grant means no host directory is bound in: the whole point of booting
        // before anything is granted.
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
        assert!(
            !argv.iter().any(|a| a == &home),
            "a fresh prefix is booted with nothing of the person's reachable"
        );
        // And the network stays off: an unshared net is what Egress::None means.
        assert!(
            argv.iter().any(|a| a == "--unshare-net"),
            "a boot has nothing to fetch"
        );
    }

    #[test]
    fn a_machine_without_wine_cannot_boot_a_prefix() {
        let err = boot_argv(
            Path::new("/var/lib/arlen/bottles/game/pfx"),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            |_| false,
        );
        assert!(
            matches!(err, Err(crate::launch::LaunchError::NoRuntime(_))),
            "answered rather than assembled: without Wine there is nothing to boot with"
        );
    }

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
        assert!(
            !b.prefix_root.join("dosdevices/z:").exists(),
            "the filesystem drive is gone"
        );
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
        assert!(
            !d.join("probe/pfx/dosdevices/y:").exists(),
            "and nothing is left in its place"
        );
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
