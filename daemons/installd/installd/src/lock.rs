//! U-1: the lock record - what is actually installed, and under what terms.
//!
//! A go.sum-style pin, one entry per installed component: the version, the
//! variant it came from, the recipe revision it was built at, when it landed,
//! and the capabilities granted at the time.
//!
//! **Every part of the update flow reads this as the OLD side.** "Is this
//! outdated" compares the catalog's version against the recorded one, and the
//! capability gate - the interruptive "this update now wants X; it didn't
//! before" - is a diff between the recorded grants and the new manifest's. The
//! manifest on disk cannot serve: it is REPLACED by an upgrade, so by the time
//! anyone asks what changed, the old side is already gone.
//!
//! **A corrupt lock is an error, never an empty one.** An empty lock reads as
//! "nothing is installed", which would make every upgrade look like a first
//! install and skip the capability gate entirely - the one moment the gate
//! exists for. A missing file IS empty, because a machine that has installed
//! nothing has nothing to record; a file that exists and does not parse is a
//! fault, and the caller has to hear about it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use arlen_forage_recipe::Capabilities;
use serde::{Deserialize, Serialize};

/// One installed component, as it stood at install time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    /// The AppStream component-id, which is what the store merges variants on.
    pub component_id: String,
    /// Which layer it was installed from (`lunpkg`, `flatpak`, `apt`).
    pub source_layer: String,
    /// The version as the source stated it.
    pub version: String,
    /// The recipe commit it was built at; empty when it did not come from one.
    #[serde(default)]
    pub recipe_commit: String,
    /// The recipe's own revision, which moves when the packaging changes but the
    /// upstream version does not.
    #[serde(default)]
    pub recipe_revision: u32,
    /// When it was installed, epoch seconds.
    pub installed_at: i64,
    /// The capabilities granted at install, in the SAME shape `diff_capabilities`
    /// compares.
    ///
    /// Stored as `Capabilities` rather than flattened to strings so the update
    /// gate can diff the recorded grants against the new manifest's directly. A
    /// string list would need a flattening on the way in and a parse on the way
    /// out, and the two would eventually disagree about what `read:x` means.
    #[serde(default)]
    pub granted: Capabilities,
}

impl LockEntry {
    /// Build an entry.
    pub fn new(
        component_id: impl Into<String>,
        source_layer: impl Into<String>,
        version: impl Into<String>,
        installed_at: i64,
        granted: Capabilities,
    ) -> Self {
        Self {
            component_id: component_id.into(),
            source_layer: source_layer.into(),
            version: version.into(),
            recipe_commit: String::new(),
            recipe_revision: 0,
            installed_at,
            granted,
        }
    }

    /// Note the recipe this was built from.
    pub fn from_recipe(mut self, commit: impl Into<String>, revision: u32) -> Self {
        self.recipe_commit = commit.into();
        self.recipe_revision = revision;
        self
    }
}

/// Why the lock could not be read or written.
#[derive(Debug)]
pub enum LockError {
    /// The file exists but does not parse. Deliberately NOT treated as empty.
    Corrupt {
        /// Where it is.
        path: PathBuf,
        /// What the parser said.
        detail: String,
    },
    /// Reading or writing failed.
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Corrupt { path, detail } => write!(
                f,
                "the install lock at {} is unreadable ({detail}); refusing to treat it as empty",
                path.display()
            ),
            LockError::Io(e) => write!(f, "the install lock could not be read or written: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

impl From<std::io::Error> for LockError {
    fn from(e: std::io::Error) -> Self {
        LockError::Io(e)
    }
}

/// The whole lock: every installed component, keyed by component-id.
///
/// One key per component rather than per (component, layer): only one variant of
/// an app is installed at a time, and which one is a FIELD of the entry. Keying
/// by both would let the same app appear twice with different grants, and
/// nothing could then say what is actually installed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lock {
    /// The entries, sorted by component-id so the file is stable across writes.
    #[serde(default)]
    pub entries: BTreeMap<String, LockEntry>,
}

impl Lock {
    /// Read the lock.
    ///
    /// A missing file is an empty lock: a machine that has installed nothing has
    /// nothing to record. A file that exists and does not parse is an error, so
    /// a damaged lock cannot quietly become "nothing is installed" and let every
    /// upgrade past the capability gate.
    pub fn load(path: &Path) -> Result<Self, LockError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(LockError::Io(e)),
        };
        toml::from_str(&text).map_err(|e| LockError::Corrupt {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })
    }

    /// The entry for a component, if it is installed.
    pub fn get(&self, component_id: &str) -> Option<&LockEntry> {
        self.entries.get(component_id)
    }

    /// Record an install or an upgrade, replacing any previous entry.
    ///
    /// An upgrade REPLACES rather than appends: the lock says what is installed
    /// now, not what has ever been installed. History belongs in the audit
    /// ledger, which is durable and append-only; a lock that grew forever would
    /// leave the diff asking which of several entries is the real old side.
    pub fn record(&mut self, entry: LockEntry) {
        self.entries.insert(entry.component_id.clone(), entry);
    }

    /// Drop a component, returning whether it was there.
    pub fn remove(&mut self, component_id: &str) -> bool {
        self.entries.remove(component_id).is_some()
    }

    /// Write the lock out atomically.
    ///
    /// Temp file plus rename, so an interrupted write cannot leave a half-written
    /// lock behind - which, being unparseable, would then refuse every read.
    pub fn save(&self, path: &Path) -> Result<(), LockError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| LockError::Io(std::io::Error::other(e.to_string())))?;

        let tmp = path.with_extension("lock.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;

        // Fsync the directory so the rename itself survives a crash: the bytes
        // being durable is no use if the name still points at the old file.
        if let Some(parent) = path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

/// Where the lock lives, beside the installed apps it describes.
pub fn lock_path() -> PathBuf {
    crate::install::user_apps_dir_pub().join("installed.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(network: &[&str], clipboard: bool) -> Capabilities {
        Capabilities {
            network: network.iter().map(|s| s.to_string()).collect(),
            clipboard,
            ..Default::default()
        }
    }

    fn entry(id: &str, version: &str, granted: Capabilities) -> LockEntry {
        LockEntry::new(id, "lunpkg", version, 1_700_000_000, granted)
    }

    fn temp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.lock");
        (dir, path)
    }

    #[test]
    fn an_entry_survives_a_write_and_a_read() {
        let (_d, path) = temp();
        let mut lock = Lock::default();
        lock.record(entry(
            "org.example.App",
            "1.2.0",
            caps(&["api.example.com"], true),
        ));
        lock.save(&path).unwrap();

        let read = Lock::load(&path).unwrap();
        let found = read.get("org.example.App").expect("should be recorded");
        assert_eq!(found.version, "1.2.0");
        assert_eq!(found.source_layer, "lunpkg");
        assert_eq!(found.installed_at, 1_700_000_000);
        assert_eq!(found.granted.network, vec!["api.example.com"]);
        assert!(found.granted.clipboard);
    }

    /// A machine that has installed nothing has nothing to record.
    #[test]
    fn a_missing_lock_is_empty_not_an_error() {
        let (_d, path) = temp();
        assert!(Lock::load(&path).unwrap().entries.is_empty());
    }

    /// The one that matters: a damaged lock must NOT read as "nothing is
    /// installed". That would make every upgrade look like a first install and
    /// skip the capability gate, which is the moment the gate exists for.
    #[test]
    fn a_corrupt_lock_is_an_error_not_an_empty_one() {
        let (_d, path) = temp();
        std::fs::write(&path, "entries = = not toml").unwrap();

        match Lock::load(&path) {
            Err(LockError::Corrupt { .. }) => {}
            Ok(lock) => panic!("a corrupt lock read as {} entries", lock.entries.len()),
            Err(other) => panic!("expected a corruption error, got {other}"),
        }
    }

    /// An upgrade replaces: the lock says what is installed, not what ever was.
    #[test]
    fn an_upgrade_replaces_the_previous_entry() {
        let mut lock = Lock::default();
        lock.record(entry("org.example.App", "1.0.0", caps(&[], true)));
        lock.record(entry("org.example.App", "2.0.0", caps(&["x.example"], true)));

        assert_eq!(lock.entries.len(), 1);
        let found = lock.get("org.example.App").unwrap();
        assert_eq!(found.version, "2.0.0");
        assert_eq!(found.granted.network, vec!["x.example"]);
    }

    /// The reason the grants are stored as `Capabilities`: the update gate can
    /// diff them against the new manifest with the already-built comparison, no
    /// conversion in between to disagree about.
    #[test]
    fn the_recorded_grants_feed_the_capability_diff_directly() {
        let mut lock = Lock::default();
        lock.record(entry("org.example.App", "1.0.0", caps(&[], false)));

        let before = &lock.get("org.example.App").unwrap().granted;
        let after = caps(&["tracker.example"], false);

        let diff = arlen_forage_capabilities::diff_capabilities(before, &after);
        assert!(
            !diff.is_empty(),
            "a newly-requested network host must show up as a widening"
        );
    }

    #[test]
    fn uninstalling_drops_the_entry() {
        let (_d, path) = temp();
        let mut lock = Lock::default();
        lock.record(entry("org.example.App", "1.0.0", caps(&[], false)));
        assert!(lock.remove("org.example.App"));
        assert!(!lock.remove("org.example.App"), "already gone");
        lock.save(&path).unwrap();
        assert!(Lock::load(&path).unwrap().get("org.example.App").is_none());
    }

    #[test]
    fn a_recipe_built_package_records_where_it_came_from() {
        let e = entry("org.example.App", "1.0.0", caps(&[], false)).from_recipe("abc123", 2);
        assert_eq!(e.recipe_commit, "abc123");
        assert_eq!(e.recipe_revision, 2);
    }

    /// Rewriting must not leave a temp file behind for the next read to trip on.
    #[test]
    fn saving_twice_leaves_only_the_lock() {
        let (dir, path) = temp();
        let mut lock = Lock::default();
        lock.record(entry("a.b.C", "1", caps(&[], false)));
        lock.save(&path).unwrap();
        lock.record(entry("d.e.F", "1", caps(&[], false)));
        lock.save(&path).unwrap();

        let files: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files, vec!["installed.lock".to_string()], "{files:?}");
        assert_eq!(Lock::load(&path).unwrap().entries.len(), 2);
    }
}
