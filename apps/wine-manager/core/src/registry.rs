//! Where bottles live between launches.
//!
//! One directory per bottle under `<data>/arlen/bottles/<id>`, holding the prefix
//! and a `bottle.toml` describing what it was granted. The id is the directory
//! name, so it is validated as a path component before it ever reaches a join.
//!
//! Two rules this module does not bend, both of them the same rule:
//!
//! A bottle that cannot be read is not a bottle with no grants. Answering an
//! unreadable `bottle.toml` with a default would hand back a bottle whose drive
//! list is empty and whose egress is closed, which looks like a safe answer and is
//! a wrong one: the next save writes that emptiness over what the person actually
//! configured.
//!
//! And a corrupt bottle is not a missing bottle. [`list_bottles`] returns what it
//! could not read alongside what it could, because a listing that quietly drops the
//! broken one tells the person their bottle is gone when it is sitting right there
//! with a typo in it.

use std::path::{Path, PathBuf};

use crate::bottle::Bottle;

/// Everything found in a bottles directory, including what would not parse.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    /// The bottles that read cleanly, sorted by id.
    pub bottles: Vec<Bottle>,
    /// The ones that did not, with the reason, so the surface can say so.
    pub unreadable: Vec<(PathBuf, String)>,
}

/// Why a bottle could not be read or written.
#[derive(Debug)]
pub enum RegistryError {
    /// The id would not be a safe directory name.
    BadId(String),
    /// There is no bottle by that name. Distinct from a bottle that is there and
    /// unreadable, which is [`RegistryError::Unreadable`].
    NoSuchBottle(PathBuf),
    /// It is there and could not be understood.
    Unreadable(PathBuf, String),
    /// The filesystem said no.
    Io(std::io::Error),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::BadId(id) => write!(f, "{id:?} cannot be a bottle name"),
            RegistryError::NoSuchBottle(p) => write!(f, "there is no bottle at {}", p.display()),
            RegistryError::Unreadable(p, why) => {
                write!(f, "{} could not be read: {why}", p.display())
            }
            RegistryError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl RegistryError {
    /// The reason WITHOUT the path.
    ///
    /// `Display` names the file, which is right when the error is shown on its
    /// own and wrong when the caller is already showing the path in the same
    /// sentence. The first render of the bottle window said the path twice and
    /// "could not be read" twice, because it composed its own message around a
    /// string that already contained one.
    pub fn detail(&self) -> String {
        match self {
            RegistryError::Unreadable(_, why) => why.clone(),
            RegistryError::NoSuchBottle(_) => "there is no bottle there".into(),
            other => other.to_string(),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<std::io::Error> for RegistryError {
    fn from(e: std::io::Error) -> Self {
        RegistryError::Io(e)
    }
}

/// Whether `id` may be a bottle name.
///
/// It becomes a directory name and part of a `WINEPREFIX`, so the rule is narrow on
/// purpose: lower-case letters, digits, dash and dot, never empty, never a dot run.
/// Refusing here means no caller has to think about `..` again.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

/// The bottles directory under a data home.
pub fn bottles_dir(data_home: &Path) -> PathBuf {
    data_home.join("arlen/bottles")
}

/// Where one bottle's description lives.
pub fn bottle_path(bottles_dir: &Path, id: &str) -> Result<PathBuf, RegistryError> {
    if !valid_id(id) {
        return Err(RegistryError::BadId(id.to_string()));
    }
    Ok(bottles_dir.join(id).join("bottle.toml"))
}

/// The prefix directory for a bottle id, which is what `WINEPREFIX` points at.
pub fn prefix_for(bottles_dir: &Path, id: &str) -> Result<PathBuf, RegistryError> {
    if !valid_id(id) {
        return Err(RegistryError::BadId(id.to_string()));
    }
    Ok(bottles_dir.join(id).join("pfx"))
}

/// Read one bottle.
///
/// Missing and unreadable are different answers, and neither of them is an empty
/// bottle.
pub fn load_bottle(bottles_dir: &Path, id: &str) -> Result<Bottle, RegistryError> {
    let path = bottle_path(bottles_dir, id)?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RegistryError::NoSuchBottle(path))
        }
        Err(e) => return Err(RegistryError::Unreadable(path, e.to_string())),
    };
    toml::from_str(&text).map_err(|e| RegistryError::Unreadable(path, e.to_string()))
}

/// Write one bottle, replacing whatever was there.
///
/// Written to a sibling and renamed, so a bottle description is never half a file:
/// a crash mid-write leaves the old one, which is a bottle that still launches.
pub fn save_bottle(bottles_dir: &Path, bottle: &Bottle) -> Result<PathBuf, RegistryError> {
    let path = bottle_path(bottles_dir, &bottle.id)?;
    let dir = path.parent().expect("bottle path always has a parent");
    std::fs::create_dir_all(dir)?;
    let text = toml::to_string_pretty(bottle)
        .map_err(|e| RegistryError::Unreadable(path.clone(), e.to_string()))?;
    let tmp = dir.join("bottle.toml.new");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Everything in the bottles directory.
///
/// A directory that does not exist yet is an empty listing, since a machine with no
/// bottles has made none. A directory entry that will not parse is reported rather
/// than skipped.
pub fn list_bottles(bottles_dir: &Path) -> Result<Listing, RegistryError> {
    let entries = match std::fs::read_dir(bottles_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(e) => return Err(e.into()),
    };
    let mut listing = Listing::default();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path().join("bottle.toml");
        if !path.exists() {
            // A prefix directory with no description: half-made, or made by hand.
            // Not a bottle and not a fault, so it is neither listed nor reported.
            continue;
        }
        match load_bottle(bottles_dir, &name) {
            Ok(b) => listing.bottles.push(b),
            Err(RegistryError::Io(e)) => return Err(RegistryError::Io(e)),
            Err(e) => listing.unreadable.push((path, e.detail())),
        }
    }
    listing.bottles.sort_by(|a, b| a.id.cmp(&b.id));
    listing.unreadable.sort();
    Ok(listing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottle::{Bottle, Egress};
    use crate::{Access, PathGrant};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("arlen-reg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bottle(id: &str) -> Bottle {
        Bottle {
            id: id.into(),
            prefix_root: PathBuf::from("/home/u/.local/share/arlen/bottles").join(id).join("pfx"),
            grants: vec![PathGrant {
                host: PathBuf::from("/home/u/Projects"),
                access: Access::ReadWrite,
            }],
            egress: Egress::None,
            plumbing: Default::default(),
        }
    }

    #[test]
    fn a_bottle_survives_being_written_and_read_back() {
        let d = dir("roundtrip");
        save_bottle(&d, &bottle("notepadpp")).unwrap();
        assert_eq!(load_bottle(&d, "notepadpp").unwrap(), bottle("notepadpp"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_written_file_reads_like_every_other_arlen_config() {
        // Enum spellings go through serde, and the first written bottle came out
        // with `access = "ReadWrite"` beside `egress = "none"`. A config file that
        // switches conventions mid-table is one a person has to guess at.
        let d = dir("spelling");
        let path = save_bottle(&d, &bottle("spelled")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("access = \"read_write\""), "{text}");
        assert!(text.contains("egress = \"none\""), "{text}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_corrupt_bottle_is_refused_and_left_on_disk() {
        // The whole point. A default here would be a bottle with no grants, and the
        // next save would write that over what the person configured.
        let d = dir("corrupt");
        let path = save_bottle(&d, &bottle("broken")).unwrap();
        std::fs::write(&path, "id = ").unwrap();
        assert!(matches!(load_bottle(&d, "broken"), Err(RegistryError::Unreadable(..))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "id = ");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_bottle_that_is_not_there_reads_differently_from_one_that_is_broken() {
        let d = dir("absent");
        assert!(matches!(load_bottle(&d, "never-made"), Err(RegistryError::NoSuchBottle(_))));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_broken_bottle_is_listed_as_broken_rather_than_vanishing() {
        let d = dir("listing");
        save_bottle(&d, &bottle("good")).unwrap();
        let bad = save_bottle(&d, &bottle("bad")).unwrap();
        std::fs::write(&bad, "not toml at all {{{").unwrap();
        let listing = list_bottles(&d).unwrap();
        assert_eq!(listing.bottles.len(), 1);
        assert_eq!(listing.bottles[0].id, "good");
        assert_eq!(listing.unreadable.len(), 1, "telling someone their bottle is gone is worse than telling them it is broken");
        assert_eq!(listing.unreadable[0].0, bad);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_listing_reason_does_not_repeat_the_path_the_caller_already_has() {
        let d = dir("detail");
        let bad = save_bottle(&d, &bottle("bad")).unwrap();
        std::fs::write(&bad, "id = ").unwrap();
        let listing = list_bottles(&d).unwrap();
        let (path, reason) = &listing.unreadable[0];
        assert_eq!(path, &bad);
        assert!(
            !reason.contains(&bad.display().to_string()),
            "the caller shows the path; saying it again reads as two different files: {reason}"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_machine_with_no_bottles_lists_none_rather_than_failing() {
        let d = dir("empty");
        assert_eq!(list_bottles(&d.join("never-created")).unwrap(), Listing::default());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_id_that_would_climb_out_of_the_directory_is_refused() {
        for bad in ["..", ".", "", "../etc", "Notepad", "a/b", ".hidden"] {
            assert!(!valid_id(bad), "{bad:?} must not be a bottle name");
            assert!(bottle_path(Path::new("/data"), bad).is_err());
            assert!(prefix_for(Path::new("/data"), bad).is_err());
        }
        assert!(valid_id("notepad-plus.2"));
    }

    #[test]
    fn the_prefix_sits_beside_the_description() {
        let d = Path::new("/data/arlen/bottles");
        assert_eq!(prefix_for(d, "x").unwrap(), Path::new("/data/arlen/bottles/x/pfx"));
        assert_eq!(bottle_path(d, "x").unwrap(), Path::new("/data/arlen/bottles/x/bottle.toml"));
    }
}
