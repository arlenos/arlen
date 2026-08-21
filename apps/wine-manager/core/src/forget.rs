//! Removing a bottle, in a way that can be undone.
//!
//! A prefix is not a cache. It holds whatever the person installed into the
//! bottle and whatever the program saved there, and the only thing separating
//! "remove this Windows program" from "lose that work" is where the directory
//! goes. So it goes to the trash, where the file manager can put it back, and
//! the description is removed only once the prefix has actually moved.
//!
//! The order is the same one revoking uses and for the same reason: the record is
//! allowed to lag what is on disk, never to lead it. A description deleted before
//! the prefix moved would leave a directory nobody owns, invisible to the window
//! that could have offered to clean it up.

use std::path::{Path, PathBuf};

use crate::bottle::Bottle;
use crate::registry;

/// What happened to a bottle that was forgotten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forgotten {
    /// Where the prefix went, so a caller can say it rather than imply it.
    pub trashed_to: Option<PathBuf>,
    /// The description that was removed.
    pub description: PathBuf,
}

/// Why a bottle could not be forgotten.
#[derive(Debug)]
pub enum ForgetError {
    /// The prefix could not be moved to the trash, so nothing was removed.
    Trash(String),
    /// The filesystem said no.
    Io(std::io::Error),
    /// The registry refused the id.
    Registry(String),
}

impl std::fmt::Display for ForgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForgetError::Trash(e) => write!(f, "the bottle was kept, because its files could not be moved to the trash: {e}"),
            ForgetError::Io(e) => write!(f, "{e}"),
            ForgetError::Registry(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ForgetError {}

/// Move a bottle's prefix to the trash and remove its description.
///
/// `trash` is injected so the sequence can be tested without a real trash
/// directory, and so a caller on a machine with no home trash can decide what to
/// do rather than have this decide for it.
pub fn forget_bottle(
    bottles_dir: &Path,
    bottle: &Bottle,
    trash: impl Fn(&Path) -> Result<PathBuf, String>,
) -> Result<Forgotten, ForgetError> {
    let description =
        registry::bottle_path(bottles_dir, &bottle.id).map_err(|e| ForgetError::Registry(e.to_string()))?;

    // A prefix that is not there is not a reason to keep the description: the
    // bottle was recorded and never booted, or somebody removed the directory by
    // hand, and either way forgetting it is what was asked for.
    let trashed_to = if bottle.prefix_root.exists() {
        Some(trash(&bottle.prefix_root).map_err(ForgetError::Trash)?)
    } else {
        None
    };

    match std::fs::remove_file(&description) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(ForgetError::Io(e)),
    }
    // The bottle's own directory, if nothing else is left in it. Left alone when
    // something is: a file this code did not put there is not this code's to
    // remove.
    let _ = std::fs::remove_dir(description.parent().unwrap_or(bottles_dir));
    Ok(Forgotten {
        trashed_to,
        description,
    })
}

/// [`forget_bottle`] moving the prefix to this user's freedesktop trash.
pub fn forget_bottle_to_trash(bottles_dir: &Path, bottle: &Bottle) -> Result<Forgotten, ForgetError> {
    forget_bottle(bottles_dir, bottle, |p| {
        arlen_freedesktop_trash::trash_for_current_user(&p.to_string_lossy())
            .map(|slot| PathBuf::from(slot.trashed().as_str()))
            .map_err(|e| format!("{e:?}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottle::Egress;
    use crate::registry::save_bottle;
    use crate::{Access, PathGrant};

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("arlen-forget-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bottle(dir: &Path, id: &str) -> Bottle {
        Bottle {
            id: id.into(),
            prefix_root: dir.join(id).join("pfx"),
            grants: vec![PathGrant { host: PathBuf::from("/srv/a"), access: Access::ReadOnly }],
            egress: Egress::None,
            plumbing: Default::default(),
        }
    }

    #[test]
    fn forgetting_moves_the_prefix_and_removes_the_record() {
        let d = scratch("plain");
        let b = bottle(&d, "gone");
        std::fs::create_dir_all(&b.prefix_root).unwrap();
        std::fs::write(b.prefix_root.join("kept.txt"), b"work").unwrap();
        save_bottle(&d, &b).unwrap();

        let bin = scratch("bin");
        let out = forget_bottle(&d, &b, |p| {
            let to = bin.join("pfx");
            std::fs::rename(p, &to).map_err(|e| e.to_string())?;
            Ok(to)
        })
        .unwrap();

        assert_eq!(out.trashed_to.as_deref(), Some(bin.join("pfx").as_path()));
        assert!(registry::load_bottle(&d, "gone").is_err(), "the record is gone");
        assert_eq!(
            std::fs::read_to_string(bin.join("pfx/kept.txt")).unwrap(),
            "work",
            "and the work went with it rather than away",
        );
        std::fs::remove_dir_all(&d).unwrap();
        std::fs::remove_dir_all(&bin).unwrap();
    }

    #[test]
    fn a_prefix_that_will_not_move_keeps_the_whole_bottle() {
        // The failure that matters. Removing the description first would leave a
        // directory nobody owns and a window that cannot offer to clean it up.
        let d = scratch("stuck");
        let b = bottle(&d, "stuck");
        std::fs::create_dir_all(&b.prefix_root).unwrap();
        save_bottle(&d, &b).unwrap();

        let err = forget_bottle(&d, &b, |_| Err("no trash on this machine".into()));
        assert!(matches!(err, Err(ForgetError::Trash(_))), "{err:?}");
        assert!(registry::load_bottle(&d, "stuck").is_ok(), "the record is still there");
        assert!(b.prefix_root.exists(), "and so is the prefix");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_bottle_whose_prefix_is_already_gone_is_still_forgotten() {
        let d = scratch("noprefix");
        let b = bottle(&d, "noprefix");
        save_bottle(&d, &b).unwrap();
        let out = forget_bottle(&d, &b, |_| panic!("nothing to move")).unwrap();
        assert_eq!(out.trashed_to, None);
        assert!(registry::load_bottle(&d, "noprefix").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
