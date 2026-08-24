//! Writing the drive table into a prefix, and taking it away again.
//!
//! A grant becomes a symlink in `dosdevices`, which is the only way a Windows
//! program can be told about a directory. The symlink is not what makes the
//! directory reachable, the bind mount is; this module writes the name the
//! program looks the directory up by, and [`crate::bottle::unmet_drives`] is what
//! keeps the two halves honest.
//!
//! The half that matters for revoking is [`write_drives`] removing letters that
//! are no longer granted. A capability browser that can show a grant and not
//! actually withdraw it is a lie with a button, so writing the table is a
//! replacement of the whole table rather than an addition to it. `c:` and the
//! `com*` ports are left where they are: the system drive is the prefix and the
//! ports reach nothing under the bottle's own `/dev`.

use std::path::{Path, PathBuf};

use crate::{Drive, UNMAPPED_DRIVE};

/// What changed in the drive table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DriveChanges {
    /// Letters written or repointed.
    pub written: Vec<char>,
    /// Letters that were there and are not granted any more.
    pub revoked: Vec<char>,
}

/// Why the drive table could not be written.
#[derive(Debug)]
pub enum WriteError {
    /// A drive claimed the letter that is never handed out. [`crate::map_drives`]
    /// cannot produce this, so it means a caller built a [`Drive`] by hand, and
    /// refusing is better than writing the one symlink the whole design exists to
    /// not write.
    ClaimsUnmappedDrive,
    /// `dosdevices` is missing, so the prefix has not been booted.
    NoPrefix(PathBuf),
    /// The filesystem said no.
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::ClaimsUnmappedDrive => write!(
                f,
                "a drive asked for {UNMAPPED_DRIVE}:, which is the whole filesystem and is never mapped"
            ),
            WriteError::NoPrefix(p) => {
                write!(f, "{} has no dosdevices, so it has not been booted", p.display())
            }
            WriteError::Io(e) => write!(f, "the drive table could not be written: {e}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e)
    }
}

/// The letters currently in the table, ignoring the system drive and the ports.
pub fn granted_letters(prefix_root: &Path) -> std::io::Result<Vec<char>> {
    let dos = prefix_root.join("dosdevices");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dos)? {
        let name = entry?.file_name().to_string_lossy().to_string();
        // `d:` and nothing longer. `d::` is Wine's form for a raw device, which
        // this module does not write and must not delete.
        let mut chars = name.chars();
        if let (Some(letter), Some(':'), None) = (chars.next(), chars.next(), chars.next()) {
            let letter = letter.to_ascii_uppercase();
            if letter != crate::SYSTEM_DRIVE {
                found.push(letter);
            }
        }
    }
    found.sort_unstable();
    Ok(found)
}

/// Make the table say exactly `drives`, and nothing else.
///
/// Replacement, not addition: a letter present in the prefix and absent from
/// `drives` is removed, which is what makes revoking a grant real rather than
/// cosmetic.
///
/// THAT INCLUDES `Z:`. This function refuses to WRITE the filesystem drive, and
/// it clears one that is already there, because the table is meant to say exactly
/// what the grants say and `Z:` is never among them. So a revoke or a re-write
/// also closes a `Z:` somebody put back with `winecfg` - which is right, and is
/// stated here because two callers now lean on it and neither says so at its own
/// call site.
pub fn write_drives(prefix_root: &Path, drives: &[Drive]) -> Result<DriveChanges, WriteError> {
    let dos = prefix_root.join("dosdevices");
    if !dos.is_dir() {
        return Err(WriteError::NoPrefix(prefix_root.to_path_buf()));
    }
    if drives
        .iter()
        .any(|d| d.letter.to_ascii_uppercase() == UNMAPPED_DRIVE)
    {
        return Err(WriteError::ClaimsUnmappedDrive);
    }

    let mut changes = DriveChanges::default();
    for existing in granted_letters(prefix_root)? {
        if !drives.iter().any(|d| d.letter == existing) {
            std::fs::remove_file(dos.join(format!("{}:", existing.to_ascii_lowercase())))?;
            changes.revoked.push(existing);
        }
    }
    for drive in drives {
        let link = dos.join(drive.dosdevice_name());
        // Remove first: a symlink cannot be repointed in place, and a grant that
        // moved to another directory has to stop pointing at the old one.
        match std::fs::symlink_metadata(&link) {
            // A directory can sit here if a prefix was made by hand. Removing the
            // file would fail with EISDIR and take the whole drive table with it.
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&link)?,
            Ok(_) => std::fs::remove_file(&link)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        std::os::unix::fs::symlink(&drive.host, &link)?;
        changes.written.push(drive.letter);
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{map_drives, Access, PathGrant};

    /// Named per test, not per process: the tests run in parallel threads of one
    /// process, so a shared directory name has them deleting each other's prefix.
    fn prefix(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("arlen-drives-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("dosdevices")).unwrap();
        std::os::unix::fs::symlink("../drive_c", tmp.join("dosdevices/c:")).unwrap();
        std::os::unix::fs::symlink("/dev/ttyS0", tmp.join("dosdevices/com1")).unwrap();
        tmp
    }

    #[test]
    fn a_withdrawn_grant_loses_its_letter() {
        let p = prefix("withdrawn");
        let two = map_drives(&[
            PathGrant {
                host: PathBuf::from("/srv/a"),
                access: Access::ReadOnly,
            },
            PathGrant {
                host: PathBuf::from("/srv/b"),
                access: Access::ReadOnly,
            },
        ])
        .unwrap();
        assert_eq!(write_drives(&p, &two).unwrap().written, vec!['D', 'E']);
        assert_eq!(granted_letters(&p).unwrap(), vec!['D', 'E']);

        let one = map_drives(&[PathGrant {
            host: PathBuf::from("/srv/a"),
            access: Access::ReadOnly,
        }])
        .unwrap();
        let changes = write_drives(&p, &one).unwrap();
        assert_eq!(
            changes.revoked,
            vec!['E'],
            "revoking has to take the letter away"
        );
        assert_eq!(granted_letters(&p).unwrap(), vec!['D']);
        assert!(!p.join("dosdevices/e:").exists());
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn writing_the_table_clears_a_filesystem_drive_somebody_put_back() {
        // `winecfg` can add `Z:` again, and a bottle is not a bottle with it. The
        // repair and revoke paths both rely on this: neither re-cuts the drive
        // itself, they write the table and expect the table to be the truth.
        let p = prefix("z_cleared");
        std::os::unix::fs::symlink("/", p.join("dosdevices/z:")).unwrap();
        let changes = write_drives(&p, &[]).unwrap();
        assert_eq!(changes.revoked, vec![UNMAPPED_DRIVE]);
        assert!(!p.join("dosdevices/z:").exists());
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn the_system_drive_and_the_ports_are_left_alone() {
        let p = prefix("left_alone");
        write_drives(&p, &[]).unwrap();
        assert!(p
            .join("dosdevices/c:")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(p
            .join("dosdevices/com1")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(granted_letters(&p).unwrap(), Vec::<char>::new());
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn a_grant_that_moved_stops_pointing_at_the_old_directory() {
        let p = prefix("moved");
        write_drives(
            &p,
            &map_drives(&[PathGrant {
                host: PathBuf::from("/srv/old"),
                access: Access::ReadOnly,
            }])
            .unwrap(),
        )
        .unwrap();
        write_drives(
            &p,
            &map_drives(&[PathGrant {
                host: PathBuf::from("/srv/new"),
                access: Access::ReadOnly,
            }])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_link(p.join("dosdevices/d:")).unwrap(),
            PathBuf::from("/srv/new")
        );
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn a_hand_built_drive_claiming_z_is_refused() {
        let p = prefix("claims_z");
        let smuggled = Drive {
            letter: UNMAPPED_DRIVE,
            host: PathBuf::from("/"),
            access: Access::ReadOnly,
        };
        assert!(matches!(
            write_drives(&p, &[smuggled]),
            Err(WriteError::ClaimsUnmappedDrive)
        ));
        assert!(
            !p.join("dosdevices/z:").exists(),
            "and nothing was written before the refusal"
        );
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn an_unbooted_prefix_is_named_rather_than_silently_skipped() {
        let tmp = std::env::temp_dir().join(format!("arlen-nodos-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(matches!(
            write_drives(&tmp, &[]),
            Err(WriteError::NoPrefix(_))
        ));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
