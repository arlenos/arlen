//! Running an installer inside a bottle, without giving the bottle the folder it
//! came from.
//!
//! THE COPY IS THE DESIGN. The obvious way to run `~/Downloads/setup.exe` inside a
//! confinement is to grant `~/Downloads` as a drive, and that is the wrong trade
//! for a one-off: the grant outlives the install, and everything else in that
//! folder - every other download, every invoice, every unrelated installer -
//! becomes readable to a Windows program for as long as the bottle exists. So the
//! file is copied into the prefix first and run from inside, and the bottle keeps
//! the reach it was made with, which is none.
//!
//! What that costs is disk: a 300 MB installer is briefly on the machine twice.
//! What it buys is that a person who installs one thing has not silently handed
//! over the folder they keep everything in.
//!
//! The name is rebuilt rather than trusted. A caller-supplied file name reaches a
//! path join, so anything with a separator or a `..` in it is a way out of the
//! prefix; what lands inside is derived from the original and cannot be either.

use std::path::{Path, PathBuf};

/// The directory inside a bottle's C: drive where installers are put.
///
/// Under `drive_c` so a Windows program sees it as `C:\arlen-installers`, and
/// named for us so it is obvious to a person browsing the prefix that we put it
/// there rather than the software they installed.
pub const INSTALLER_DIR: &str = "drive_c/arlen-installers";

/// Why an installer could not be brought into a bottle.
#[derive(Debug)]
pub enum InstallError {
    /// The path names nothing, or names something that is not a regular file.
    NotAFile(PathBuf),
    /// The name it would land under is not one a file may have here.
    BadName(String),
    /// The copy failed.
    Io(std::io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::NotAFile(p) => write!(f, "{} is not a file to install from", p.display()),
            InstallError::BadName(n) => write!(f, "{n} is not a name an installer may have"),
            InstallError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        InstallError::Io(e)
    }
}

/// The file name an installer will be copied in under.
///
/// Derived from the original so a person recognises it, but made of only the
/// characters that cannot mean anything else: letters, digits, dot, dash and
/// underscore, with everything else folded to an underscore. A name that is empty
/// or is entirely dots after that is refused rather than replaced with an invented
/// one - the caller passed something that was never a file name.
pub fn safe_name(original: &str) -> Option<String> {
    // Only the last component, so a caller that hands over a whole path does not
    // get to choose a directory.
    let base = original.rsplit('/').next().unwrap_or(original);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(cleaned)
}

/// Copy an installer into a bottle's prefix and answer where it landed.
///
/// The answer is a host path INSIDE the prefix, which is what the confined Wine
/// will be pointed at: the same path is visible on both sides, since the prefix is
/// bound at its own location.
pub fn bring_installer_in(prefix_root: &Path, installer: &Path) -> Result<PathBuf, InstallError> {
    // `symlink_metadata` rather than `metadata`: a link that points at a directory
    // or a device should be refused as what it is, not followed into a copy that
    // would never finish.
    let meta = std::fs::symlink_metadata(installer)
        .map_err(|_| InstallError::NotAFile(installer.to_path_buf()))?;
    if !meta.is_file() {
        return Err(InstallError::NotAFile(installer.to_path_buf()));
    }
    let name = installer
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(safe_name)
        .ok_or_else(|| InstallError::BadName(installer.to_string_lossy().into_owned()))?;

    let dir = prefix_root.join(INSTALLER_DIR);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&name);
    std::fs::copy(installer, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arlen-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_name_that_would_leave_the_directory_cannot() {
        assert_eq!(safe_name("setup.exe").as_deref(), Some("setup.exe"));
        assert_eq!(safe_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_name("a/b/c.exe").as_deref(), Some("c.exe"));
        // Kept recognisable, with everything that is not a plain name folded away.
        assert_eq!(
            safe_name("My Game (Setup).exe").as_deref(),
            Some("My_Game__Setup_.exe")
        );
        assert_eq!(safe_name(".."), None);
        assert_eq!(safe_name(""), None);
    }

    #[test]
    fn an_installer_is_copied_in_rather_than_reached_out_to() {
        let downloads = scratch("downloads");
        let installer = downloads.join("setup.exe");
        std::fs::write(&installer, b"MZ fake installer").unwrap();
        // Something else in the same folder, which is the whole reason not to grant
        // it: a bottle that could read the installer could read this too.
        std::fs::write(downloads.join("tax-return.pdf"), b"private").unwrap();

        let prefix = scratch("prefix");
        let landed = bring_installer_in(&prefix, &installer).unwrap();

        assert_eq!(landed, prefix.join(INSTALLER_DIR).join("setup.exe"));
        assert_eq!(std::fs::read(&landed).unwrap(), b"MZ fake installer");
        assert!(
            landed.starts_with(&prefix),
            "what the bottle runs is inside the bottle"
        );
        assert!(
            downloads.join("tax-return.pdf").is_file(),
            "the folder it came from is untouched and ungranted"
        );
    }

    #[test]
    fn a_directory_or_a_missing_path_is_refused_rather_than_copied() {
        let prefix = scratch("refuse-prefix");
        let dir = scratch("refuse-dir");
        assert!(matches!(
            bring_installer_in(&prefix, &dir),
            Err(InstallError::NotAFile(_))
        ));
        assert!(matches!(
            bring_installer_in(&prefix, Path::new("/nonexistent/setup.exe")),
            Err(InstallError::NotAFile(_))
        ));
        assert!(
            !prefix.join(INSTALLER_DIR).exists(),
            "a refusal makes no directories: nothing was attempted"
        );
    }
}
