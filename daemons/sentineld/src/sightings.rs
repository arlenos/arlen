//! The encrypted sighting store (`tracker-sentinel-plan.md` §2.1, §6 job 2).
//!
//! SEN-4 needs a tag's separated sightings to survive a suspend, because the
//! criteria are about a span across awake-sessions. That history is raw location
//! plus BLE identity, which is the single most sensitive thing this daemon touches,
//! so where it goes is a design decision rather than a storage one:
//!
//! **NOT the Knowledge Graph.** That is the anti-Recall invariant the plan states
//! outright. The graph is bulk-readable by anything holding a read scope; a log of
//! where you were and what was near you must not become another queryable table.
//! Only the finished, coarse alert event goes to the KG and the audit ledger.
//!
//! **Encrypted at rest, under this daemon's own master.** A local file at 0600 in a
//! 0700 directory is already the custody every Arlen daemon key uses; the encryption
//! is what makes a stolen disk, a careless backup or a sync client carrying the
//! state directory away not also carry a month of somebody's movements. The
//! primitive is the reviewed [`arlen_secret_vault::Vault`] rather than a second
//! hand-rolled AEAD, and the master is the reviewed `BuilderKey` custody, for the
//! same reason.
//!
//! **The filename is a keyed hash, not the correlator.** A record sealed under its
//! tag's own identifier would leave that identifier lying in a directory listing,
//! and the directory listing is exactly what an attacker with a moment at your
//! machine reads. So the record id is HMAC-SHA256 over the correlator under the
//! daemon's master: stable (the same tag finds its own record), opaque (the listing
//! says how many tags, never which), and unusable for correlating two machines.
//!
//! **Nothing is kept past the detection window.** A correlator is unchainable across
//! its rotation by design, so a tag older than [`MAX_WINDOW_SECS`] can never
//! contribute to an alert again - keeping it would be collecting location history
//! for no purpose, which is the thing this daemon exists to object to when other
//! software does it. [`SightingStore::prune`] is what makes that true rather than
//! stated.

use std::path::{Path, PathBuf};

use arlen_forage_signing::BuilderKey;
use arlen_secret_vault::{Vault, VaultError};
use arlen_sentinel_detect::home_anchor::HomeAnchor;
use arlen_sentinel_detect::sighting::{Sighting, TrackedTag, MAX_WINDOW_SECS};
use arlen_sentinel_detect::tracker::TrackerBrand;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The record the home anchor lives in. A fixed name rather than a hashed one: it
/// is the store's own state, not a tag, and `prune` must be able to tell the two
/// apart - a listing that could not would eventually delete the anchor as if it
/// were an expired tag.
const ANCHOR_RECORD: &str = "home-anchor";

/// The HMAC domain, so this key derivation cannot collide with another use of the
/// same master.
const ID_DOMAIN: &[u8] = b"arlen-tracker-sentinel-record-id";

/// Why the store could not do what was asked. Every variant means nothing was
/// written and no sighting history was produced.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// No state directory could be resolved (no `XDG_STATE_HOME`, no home).
    #[error("no state directory for the sighting store")]
    NoStateDir,
    /// The master secret could not be loaded or created.
    #[error("sighting store master: {0}")]
    Master(String),
    /// A filesystem error outside the vault's own writes.
    #[error("sighting store io: {0}")]
    Io(String),
    /// The vault refused (corrupt record, failed decrypt, bad id).
    #[error("sighting store vault: {0}")]
    Vault(#[from] VaultError),
    /// A record decrypted but did not parse as a tag.
    #[error("a sighting record did not parse")]
    Malformed,
}

/// `$XDG_STATE_HOME/arlen/tracker-sentinel`, else `~/.local/state/...`.
pub fn sightings_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))?;
    Some(base.join("arlen").join("tracker-sentinel"))
}

/// The per-tag encrypted store.
pub struct SightingStore {
    vault: Vault,
    master: [u8; 32],
    dir: PathBuf,
}

impl SightingStore {
    /// Open (creating on first use) the store under `dir`.
    ///
    /// The directory is made 0700 before the master lands in it, not after: a key
    /// written into a world-readable directory is readable for however long the
    /// window between the two is.
    pub fn open_in(dir: &Path) -> Result<Self, StoreError> {
        ensure_private_dir(dir)?;
        let key = BuilderKey::load_or_create(&dir.join("master.key"))
            .map_err(|e| StoreError::Master(e.to_string()))?;
        // The persisted CSPRNG seed IS the symmetric master; the Ed25519 wrapper
        // around it is incidental here (this daemon never signs). Same reading the
        // account daemon's vault master takes of the same primitive.
        let master: [u8; 32] = key.signing_key().to_bytes();
        Ok(Self { vault: Vault::new(master, dir.join("tags")), master, dir: dir.to_path_buf() })
    }

    /// Open the store at its resolved state directory.
    pub fn open_default() -> Result<Self, StoreError> {
        let dir = sightings_dir().ok_or(StoreError::NoStateDir)?;
        Self::open_in(&dir)
    }

    /// Where the records live (the `tags` subdirectory).
    fn records_dir(&self) -> PathBuf {
        self.dir.join("tags")
    }

    /// The opaque record id for a correlator: HMAC-SHA256 under the master, hex.
    ///
    /// Keyed rather than plain: a plain digest of an Apple public key is as
    /// correlatable across machines as the key itself, because anybody who saw the
    /// same advert can compute it. Under this machine's master it is meaningless
    /// anywhere else.
    fn record_id(&self, correlator: &[u8]) -> String {
        let mut mac = <Hmac<Sha256>>::new_from_slice(&self.master)
            .expect("HMAC-SHA256 accepts a 32-byte key");
        mac.update(ID_DOMAIN);
        mac.update(correlator);
        mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The tag this correlator belongs to, or `None` if it has not been seen (or
    /// its record was pruned).
    pub fn load(&self, correlator: &[u8]) -> Result<Option<TrackedTag>, StoreError> {
        let Some(bytes) = self.vault.load(&self.record_id(correlator))? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|_| StoreError::Malformed)
    }

    /// Persist a tag.
    pub fn save(&self, tag: &TrackedTag) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(tag).map_err(|_| StoreError::Malformed)?;
        self.vault.store(&self.record_id(&tag.correlator), &bytes)?;
        Ok(())
    }

    /// Record one separated sighting against its correlator, opening a tag for it
    /// if this is the first, and hand back the updated tag.
    ///
    /// The read-modify-write is the whole point of the store: the criteria are about
    /// a history, and a daemon that only ever saw the current scan has none.
    pub fn record(
        &self,
        correlator: &[u8],
        brand: TrackerBrand,
        sighting: Sighting,
    ) -> Result<TrackedTag, StoreError> {
        let mut tag = self
            .load(correlator)?
            .unwrap_or_else(|| TrackedTag::new(correlator.to_vec(), brand, sighting.seen_at_secs));
        tag.record(sighting);
        self.save(&tag)?;
        Ok(tag)
    }

    /// The learned home anchor, or a fresh one if none has been kept yet.
    ///
    /// It lives in the same sealed store as the tags and for the same reason: where
    /// somebody sleeps is the most identifying coordinate on the machine, and it
    /// would be an odd kind of care to encrypt the tags that passed the house and
    /// leave the house itself in the clear beside them.
    pub fn anchor(&self) -> Result<HomeAnchor, StoreError> {
        match self.vault.load(ANCHOR_RECORD)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| StoreError::Malformed),
            None => Ok(HomeAnchor::default()),
        }
    }

    /// Persist the anchor.
    pub fn save_anchor(&self, anchor: &HomeAnchor) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(anchor).map_err(|_| StoreError::Malformed)?;
        self.vault.store(ANCHOR_RECORD, &bytes)?;
        Ok(())
    }

    /// Forget one tag outright - what "this is mine, stop watching it" writes.
    pub fn forget(&self, correlator: &[u8]) -> Result<(), StoreError> {
        self.vault.remove(&self.record_id(correlator))?;
        Ok(())
    }

    /// Every stored tag, with the record id it was read from.
    ///
    /// A record that fails to decrypt or parse is SKIPPED rather than failing the
    /// sweep: one corrupt file must not make the daemon blind to every other tag,
    /// and there is nothing a caller could do with the failure that dropping it does
    /// not already do.
    pub fn all(&self) -> Result<Vec<(String, TrackedTag)>, StoreError> {
        let dir = self.records_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // No directory means nothing stored, which is the ordinary first run.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StoreError::Io(e.to_string())),
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "vault"))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter(|id| id != ANCHOR_RECORD)
            .collect();
        ids.sort();
        let mut out = Vec::new();
        for id in ids {
            if let Ok(Some(bytes)) = self.vault.load(&id) {
                if let Ok(tag) = serde_json::from_slice::<TrackedTag>(&bytes) {
                    out.push((id, tag));
                }
            }
        }
        Ok(out)
    }

    /// Delete every tag whose window closed before `now_secs`, and say how many
    /// went.
    ///
    /// Not a cache eviction: a correlator cannot be chained across its rotation, so
    /// a tag past its window can never contribute to an alert again and keeping it
    /// would be holding location history for nothing. Run it on start and on a
    /// timer, so a machine that was off for a week comes back without a week of
    /// stale positions in it.
    pub fn prune(&self, now_secs: u64) -> Result<usize, StoreError> {
        let mut gone = 0;
        for (id, tag) in self.all()? {
            if now_secs.saturating_sub(tag.window_opened_secs) >= MAX_WINDOW_SECS {
                self.vault.remove(&id)?;
                gone += 1;
            }
        }
        Ok(gone)
    }
}

/// Create `dir` (and its parents) and clamp it to 0700.
fn ensure_private_dir(dir: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| StoreError::Io(e.to_string()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| StoreError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_sentinel_detect::movement::Fix;

    fn sighting(secs: u64, lat: f64, lon: f64, epoch: u64) -> Sighting {
        Sighting { seen_at_secs: secs, fix: Fix { lat, lon }, epoch_id: epoch }
    }

    fn store(dir: &Path) -> SightingStore {
        SightingStore::open_in(dir).expect("a fresh store opens")
    }

    #[test]
    fn a_tag_survives_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let correlator = b"apple-public-key".to_vec();
        {
            let s = store(tmp.path());
            s.record(&correlator, TrackerBrand::AppleFindMy, sighting(1_000, 47.26, 11.40, 1))
                .unwrap();
            s.record(&correlator, TrackerBrand::AppleFindMy, sighting(4_700, 47.26, 11.45, 2))
                .unwrap();
        }
        // A second open is a second daemon start: same directory, same master.
        let s = store(tmp.path());
        let tag = s.load(&correlator).unwrap().expect("the tag is still there");
        assert_eq!(tag.len(), 2);
        assert_eq!(tag.observe(5_000).distinct_epochs, 2);
    }

    /// The listing says how many tags, never which.
    #[test]
    fn the_correlator_is_not_in_the_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.record(b"apple-public-key", TrackerBrand::AppleFindMy, sighting(1_000, 47.26, 11.40, 1))
            .unwrap();
        let names: Vec<String> = std::fs::read_dir(tmp.path().join("tags"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(!names[0].contains("apple"), "{names:?}");
        assert!(names[0].ends_with(".vault"));
    }

    /// What is on disk is ciphertext: the location a person was at must not be
    /// readable by opening the file.
    #[test]
    fn the_record_is_not_readable_without_the_master() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.record(b"tag", TrackerBrand::Tile, sighting(1_000, 47.2692, 11.4041, 1)).unwrap();
        let file = std::fs::read_dir(tmp.path().join("tags")).unwrap().flatten().next().unwrap();
        let bytes = std::fs::read(file.path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("47.26"), "the fix is in the clear");
        assert!(!text.contains("seen_at_secs"), "the shape is in the clear");
    }

    #[test]
    fn a_tag_past_its_window_is_pruned_and_a_live_one_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.record(b"old", TrackerBrand::Tile, sighting(1_000, 47.26, 11.40, 1)).unwrap();
        s.record(b"new", TrackerBrand::Tile, sighting(1_000, 47.26, 11.40, 1)).unwrap();

        let now = 1_000 + MAX_WINDOW_SECS;
        // Keep "new" alive by recording inside the window, which re-opens it.
        s.record(b"new", TrackerBrand::Tile, sighting(now, 47.26, 11.40, 2)).unwrap();

        assert_eq!(s.prune(now).unwrap(), 1);
        assert!(s.load(b"old").unwrap().is_none());
        assert!(s.load(b"new").unwrap().is_some());
    }

    #[test]
    fn the_anchor_survives_a_restart_and_is_not_a_tag() {
        use arlen_sentinel_detect::home_anchor::NightFix;
        let tmp = tempfile::tempdir().unwrap();
        let here = Fix { lat: 47.2692, lon: 11.4041 };
        {
            let s = store(tmp.path());
            let mut a = s.anchor().unwrap();
            for night in 1..=3 {
                a.record(NightFix { night, fix: here });
            }
            s.save_anchor(&a).unwrap();
            s.record(b"tag", TrackerBrand::Tile, sighting(1_000, 47.26, 11.40, 1)).unwrap();
        }
        let s = store(tmp.path());
        assert!(s.anchor().unwrap().is_home(here));
        // The listing is tags, and the anchor is not one of them.
        assert_eq!(s.all().unwrap().len(), 1);
    }

    /// A prune must never take the anchor with it: it has no window to expire.
    #[test]
    fn pruning_leaves_the_anchor_alone() {
        use arlen_sentinel_detect::home_anchor::{HomeAnchor, NightFix};
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let here = Fix { lat: 47.2692, lon: 11.4041 };
        let mut a = HomeAnchor::default();
        for night in 1..=3 {
            a.record(NightFix { night, fix: here });
        }
        s.save_anchor(&a).unwrap();
        s.record(b"old", TrackerBrand::Tile, sighting(1_000, 47.26, 11.40, 1)).unwrap();

        assert_eq!(s.prune(1_000 + MAX_WINDOW_SECS).unwrap(), 1);
        assert!(s.anchor().unwrap().is_home(here), "the anchor is still there");
    }

    #[test]
    fn forgetting_a_tag_removes_it() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.record(b"mine", TrackerBrand::SamsungSmartTag, sighting(1_000, 47.26, 11.40, 1)).unwrap();
        s.forget(b"mine").unwrap();
        assert!(s.load(b"mine").unwrap().is_none());
        assert!(s.all().unwrap().is_empty());
    }

    #[test]
    fn an_unseen_correlator_reads_as_absent_not_as_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(store(tmp.path()).load(b"never-seen").unwrap().is_none());
    }

    #[test]
    fn the_store_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let _ = store(tmp.path());
        let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{mode:o}");
    }
}
