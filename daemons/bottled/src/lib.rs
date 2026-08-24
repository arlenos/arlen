//! What a Wine prefix reaches on its own, and the drive map a capability-scoped
//! bottle gives it instead (`wine-proton-plan.md`).
//!
//! The plan's whole claim is that a Windows program should reach exactly what it
//! was granted and nothing else. That claim has a specific enemy, and it is not
//! an attacker: it is `wineboot`, which wires a fresh prefix into the real home
//! as a FEATURE. Measured on wine-11.14 with `find ~/.wine -type l`, a prefix
//! nobody has touched carries eight symlinks that leave it:
//!
//! ```text
//! dosdevices/z:                    -> /
//! drive_c/users/<user>/Desktop     -> /home/<user>/Desktop
//! drive_c/users/<user>/Documents   -> /home/<user>/Documents
//! drive_c/users/<user>/Downloads   -> /home/<user>/Downloads
//! drive_c/users/<user>/Music       -> /home/<user>/Music
//! drive_c/users/<user>/Pictures    -> /home/<user>/Pictures
//! drive_c/users/<user>/Videos      -> /home/<user>/Videos
//! dosdevices/c:                    -> ../drive_c   (contained, the system drive)
//! ```
//!
//! So "delete `Z:` and you are isolated" is wrong by six links. `Z:` is the one
//! everybody names because it is the visible one in `winecfg`; the user-shell
//! folders are the ones that actually hold the documents, and they are separate
//! symlinks that survive removing the drive. Both removals are operations Wine
//! supports (the Drives tab and the Desktop Integration tab do exactly this), so
//! severing them is using Wine as designed rather than fighting it.
//!
//! Two things live here, both pure so they can be tested without Wine present:
//! reading a prefix's links for where they actually go ([`reach`], [`escapes`]),
//! and turning a grant list into drive letters ([`map_drives`]).

pub mod bottle;
pub mod create;
pub mod dosdevices;
pub mod forget;
pub mod health;
pub mod launch;
pub mod plumbing;
pub mod protocol;
pub mod registry;
pub mod sever;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a symlink found inside a prefix actually points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// Stays inside the prefix. `dosdevices/c: -> ../drive_c` is this, and it is
    /// the only one of the defaults that is fine as it stands.
    Contained,
    /// The entire filesystem. Only `dosdevices/z:` is this by default, and it is
    /// the reason a native prefix is not a boundary at all.
    Filesystem,
    /// A device node. `wineboot` writes `com1..com32 -> /dev/ttyS0..31` whether or
    /// not the machine has a serial port. Under a bottle these dangle rather than
    /// reach anything, which is why they are classified apart from [`Reach::Host`]
    /// instead of being reported as an escape into the user's data.
    Device(PathBuf),
    /// Somewhere else on the host: the user's real Documents, Downloads and the
    /// rest of the shell folders.
    Host(PathBuf),
}

/// One symlink that leaves the prefix, with where it goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Escape {
    /// The link itself, as found under the prefix.
    pub link: PathBuf,
    /// What it points at.
    pub target: PathBuf,
    /// The classification [`reach`] gave it.
    pub reach: Reach,
}

/// Resolve a link target the way the kernel would, without following anything.
///
/// `dosdevices/c:` is `../drive_c`, a RELATIVE target, so a target cannot be
/// classified without knowing where the link sits. This is lexical on purpose:
/// `canonicalize` would resolve symlinks along the way, and a scan meant to find
/// out what a link reaches must not walk through other links to answer.
///
/// The residual is stated rather than closed: if `drive_c` were itself replaced
/// by a link out of the prefix, this returns "contained" and the confinement, not
/// this scan, is what still bounds the program. The severing is the second line.
pub fn resolve_target(link: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        return target.to_path_buf();
    }
    let mut out = link.parent().unwrap_or(Path::new("/")).to_path_buf();
    for part in target.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Classify one link against the prefix it was found in.
///
/// Both paths are compared by components, never as strings: `/home/u/.wine-old`
/// begins with `/home/u/.wine` as text and is a different directory on disk, so a
/// string prefix test would report an escape as contained. That is the wrong way
/// round for a boundary check, which is why this is a test below and not a note.
pub fn reach(prefix_root: &Path, link: &Path, target: &Path) -> Reach {
    let target = resolve_target(link, target);
    if target.starts_with(prefix_root) {
        return Reach::Contained;
    }
    if target == Path::new("/") {
        return Reach::Filesystem;
    }
    if target.starts_with("/dev") {
        return Reach::Device(target);
    }
    Reach::Host(target)
}

/// Every link in `links` that leaves `prefix_root`, in the order given.
///
/// Takes the listing rather than walking the filesystem so the classification is
/// testable against a prefix that is not on this machine, and so the caller can
/// decide how to walk (a bottle scan follows no symlinks; a repair pass might).
pub fn escapes(prefix_root: &Path, links: &[(PathBuf, PathBuf)]) -> Vec<Escape> {
    links
        .iter()
        .filter_map(|(link, target)| {
            let reach = reach(prefix_root, link, target);
            (reach != Reach::Contained).then(|| Escape {
                link: link.clone(),
                target: target.clone(),
                reach,
            })
        })
        .collect()
}

/// Whether a grant may write, or only read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// The program sees the files and cannot change them.
    ReadOnly,
    /// The program may write, and an undo of that is the file manager's problem,
    /// not this crate's.
    ReadWrite,
}

/// One host directory a bottle was granted, and on what terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathGrant {
    /// The host directory, absolute.
    pub host: PathBuf,
    /// Read or write.
    pub access: Access,
}

/// A granted directory as the Windows program sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drive {
    /// The drive letter, upper case (`dosdevices` holds the lower-case form).
    pub letter: char,
    /// The host directory it maps to.
    pub host: PathBuf,
    /// Read or write, carried through so the bind mount and the drive agree.
    pub access: Access,
}

impl Drive {
    /// The `dosdevices` entry name for this drive, which Wine expects lower case
    /// with the colon: `d:`.
    pub fn dosdevice_name(&self) -> String {
        format!("{}:", self.letter.to_ascii_lowercase())
    }
}

/// Why a grant list could not be turned into drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveError {
    /// A grant path was relative. A relative grant has no meaning here: the
    /// symlink written into `dosdevices` would resolve against the prefix.
    NotAbsolute(PathBuf),
    /// More grants than there are letters between D and Y.
    NoLettersLeft { granted: usize, available: usize },
}

impl std::fmt::Display for DriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriveError::NotAbsolute(p) => {
                write!(
                    f,
                    "{} is not an absolute path, so it cannot be a drive",
                    p.display()
                )
            }
            DriveError::NoLettersLeft { granted, available } => write!(
                f,
                "{granted} directories were granted and a bottle has {available} drive letters"
            ),
        }
    }
}

impl std::error::Error for DriveError {}

/// The system drive, always the prefix's own `drive_c`.
pub const SYSTEM_DRIVE: char = 'C';

/// The letter a bottle never hands out.
///
/// Not because the letter is special, but because a program that finds `Z:` and
/// walks it expects the filesystem to be there. Leaving it unmapped is the point
/// of the whole design, so it is a constant rather than a rule in one function.
pub const UNMAPPED_DRIVE: char = 'Z';

/// The letters a grant may be given: D through Y.
///
/// A and B are skipped because Windows programs still treat them as removable
/// media and will offer to format them; C is the prefix; Z is [`UNMAPPED_DRIVE`].
pub fn grantable_letters() -> Vec<char> {
    ('D'..='Y').collect()
}

/// Turn a grant list into drive letters.
///
/// Sorted by host path, not by the order the grants arrived. A Windows program
/// writes the paths it was shown into its own settings, so if the same grant set
/// produced `D:` one launch and `E:` the next, the program's own saved paths would
/// break each time the user added an unrelated folder. Stable letters cost one
/// sort and save that.
///
/// A directory granted twice collapses to one drive with the wider access, so a
/// caller that merges a per-app grant with a per-bottle default does not get two
/// letters for one folder.
pub fn map_drives(grants: &[PathGrant]) -> Result<Vec<Drive>, DriveError> {
    for g in grants {
        if !g.host.is_absolute() {
            return Err(DriveError::NotAbsolute(g.host.clone()));
        }
    }

    let mut merged: Vec<PathGrant> = Vec::new();
    for g in grants {
        match merged.iter_mut().find(|m| m.host == g.host) {
            Some(existing) => existing.access = existing.access.max(g.access),
            None => merged.push(g.clone()),
        }
    }
    merged.sort_by(|a, b| a.host.cmp(&b.host));

    let letters = grantable_letters();
    if merged.len() > letters.len() {
        return Err(DriveError::NoLettersLeft {
            granted: merged.len(),
            available: letters.len(),
        });
    }

    Ok(merged
        .into_iter()
        .zip(letters)
        .map(|(g, letter)| Drive {
            letter,
            host: g.host,
            access: g.access,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// The eight links a fresh wine-11.14 prefix carries, read off this machine's
    /// own `~/.wine` on 20 August. The com* links are represented by one of the
    /// thirty-two, since they are all the same shape.
    fn default_prefix_links() -> Vec<(PathBuf, PathBuf)> {
        vec![
            // Relative, as wineboot actually writes it.
            (p("/home/u/.wine/dosdevices/c:"), p("../drive_c")),
            (p("/home/u/.wine/dosdevices/z:"), p("/")),
            (p("/home/u/.wine/dosdevices/com1"), p("/dev/ttyS0")),
            (
                p("/home/u/.wine/drive_c/users/u/Desktop"),
                p("/home/u/Desktop"),
            ),
            (
                p("/home/u/.wine/drive_c/users/u/Documents"),
                p("/home/u/Documents"),
            ),
            (
                p("/home/u/.wine/drive_c/users/u/Downloads"),
                p("/home/u/Downloads"),
            ),
            (p("/home/u/.wine/drive_c/users/u/Music"), p("/home/u/Music")),
            (
                p("/home/u/.wine/drive_c/users/u/Pictures"),
                p("/home/u/Pictures"),
            ),
            (
                p("/home/u/.wine/drive_c/users/u/Videos"),
                p("/home/u/Videos"),
            ),
        ]
    }

    #[test]
    fn removing_the_z_drive_leaves_six_ways_into_the_home() {
        let found = escapes(&p("/home/u/.wine"), &default_prefix_links());
        let into_home: Vec<_> = found
            .iter()
            .filter(|e| matches!(e.reach, Reach::Host(_)))
            .map(|e| e.link.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            into_home,
            [
                "Desktop",
                "Documents",
                "Downloads",
                "Music",
                "Pictures",
                "Videos"
            ],
            "the shell folders are separate symlinks and survive dropping Z:"
        );
        assert_eq!(
            found
                .iter()
                .filter(|e| e.reach == Reach::Filesystem)
                .count(),
            1,
            "Z: is one of the eight, not the whole problem"
        );
    }

    #[test]
    fn the_system_drive_is_the_only_default_link_that_stays_inside() {
        let all = default_prefix_links();
        let contained: Vec<_> = all
            .iter()
            .filter(|(l, t)| reach(&p("/home/u/.wine"), l, t) == Reach::Contained)
            .collect();
        assert_eq!(contained.len(), 1);
        assert!(contained[0].0.ends_with("c:"));
    }

    #[test]
    fn the_system_drive_is_relative_and_resolving_it_wrong_would_delete_the_prefix() {
        // `dosdevices/c: -> ../drive_c`. Read as an absolute path it does not
        // begin with the prefix, so it classifies as an escape into the host, and
        // the severing pass would replace the system drive with an empty
        // directory. A real prefix on disk caught this; the fixture had written
        // c: as absolute, which no wineboot has ever done.
        assert_eq!(
            reach(
                &p("/home/u/.wine"),
                &p("/home/u/.wine/dosdevices/c:"),
                &p("../drive_c")
            ),
            Reach::Contained
        );
    }

    #[test]
    fn a_sibling_directory_sharing_the_name_is_not_contained() {
        // /home/u/.wine-old begins with /home/u/.wine as text. Comparing strings
        // here would report a link into a different prefix as safely inside this
        // one, which is the wrong direction for a boundary check to be wrong in.
        assert_eq!(
            reach(
                &p("/home/u/.wine"),
                &p("/home/u/.wine/dosdevices/d:"),
                &p("/home/u/.wine-old/drive_c")
            ),
            Reach::Host(p("/home/u/.wine-old/drive_c"))
        );
    }

    #[test]
    fn serial_devices_are_not_reported_as_reaching_the_users_files() {
        assert_eq!(
            reach(
                &p("/home/u/.wine"),
                &p("/home/u/.wine/dosdevices/com1"),
                &p("/dev/ttyS0")
            ),
            Reach::Device(p("/dev/ttyS0"))
        );
    }

    #[test]
    fn the_letters_start_after_the_system_drive_and_stop_before_z() {
        let letters = grantable_letters();
        assert_eq!(letters.first(), Some(&'D'));
        assert_eq!(letters.last(), Some(&'Y'));
        assert!(!letters.contains(&SYSTEM_DRIVE));
        assert!(
            !letters.contains(&UNMAPPED_DRIVE),
            "a bottle that hands out Z: has given away the filesystem"
        );
    }

    #[test]
    fn the_same_grants_get_the_same_letters_whatever_order_they_arrive_in() {
        let a = PathGrant {
            host: p("/home/u/Projects"),
            access: Access::ReadWrite,
        };
        let b = PathGrant {
            host: p("/home/u/Documents"),
            access: Access::ReadOnly,
        };
        let one = map_drives(&[a.clone(), b.clone()]).unwrap();
        let other = map_drives(&[b, a]).unwrap();
        assert_eq!(one, other);
        assert_eq!(one[0].host, p("/home/u/Documents"));
        assert_eq!(one[0].letter, 'D');
    }

    #[test]
    fn granting_one_directory_twice_takes_the_wider_access() {
        let drives = map_drives(&[
            PathGrant {
                host: p("/srv/share"),
                access: Access::ReadOnly,
            },
            PathGrant {
                host: p("/srv/share"),
                access: Access::ReadWrite,
            },
        ])
        .unwrap();
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].access, Access::ReadWrite);
    }

    #[test]
    fn a_relative_grant_is_refused_rather_than_resolved() {
        let err = map_drives(&[PathGrant {
            host: p("Documents"),
            access: Access::ReadOnly,
        }]);
        assert_eq!(err, Err(DriveError::NotAbsolute(p("Documents"))));
    }

    #[test]
    fn more_grants_than_letters_is_an_error_and_not_a_silent_truncation() {
        let grants: Vec<_> = (0..30)
            .map(|i| PathGrant {
                host: p(&format!("/srv/{i:02}")),
                access: Access::ReadOnly,
            })
            .collect();
        assert_eq!(
            map_drives(&grants),
            Err(DriveError::NoLettersLeft {
                granted: 30,
                available: 22
            })
        );
    }

    #[test]
    fn a_drive_names_its_dosdevices_entry_in_the_form_wine_reads() {
        let d = Drive {
            letter: 'D',
            host: p("/srv/share"),
            access: Access::ReadOnly,
        };
        assert_eq!(d.dosdevice_name(), "d:");
    }
}
