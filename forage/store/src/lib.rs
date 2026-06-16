//! Content-addressed, refcounted local store for forage.
//!
//! Source trees and built artifacts are stored under their sha256 content
//! address, so identical content is stored once and a fetch can be verified
//! against a recipe's declared `sha256`/commit. The store is refcounted per
//! owner (an app id): an uninstall drops that owner's references and
//! [`Store::gc`] collects only objects no owner still references, so a
//! still-used dependency survives (forage-recipes.md sections 13, 17a).
//!
//! Layout under the store root:
//! ```text
//! lock                         flock'd by every mutating operation
//! objects/<ab>/<full-hash>     the content, sharded by the first two hex chars
//! refs/<full-hash>/<owner>     one empty marker per owner holding a reference
//! ```
//! Refs are per-owner marker files rather than a mutable counter, so a crash
//! mid-operation cannot corrupt a count: the refcount is the number of marker
//! files that exist.
//!
//! All mutating operations (`put*`, `add_ref`, `release`, `gc`) take an
//! exclusive interprocess lock so a GC pass can never race a reference being
//! added; reads are content-addressed and lock-free. [`Store::put_referenced`]
//! stores and roots an object in one lock hold, so a just-fetched object is
//! never collectable before it is referenced.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use fs2::FileExt;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A failure interacting with the store.
#[derive(Debug, Error)]
pub enum StoreError {
    /// An underlying filesystem error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The requested object is not in the store.
    #[error("object {0} not found")]
    NotFound(String),
    /// Stored content did not match its address (corruption or tampering).
    #[error("integrity: object {expected} hashes to {actual}")]
    Integrity {
        /// The address the object is stored under.
        expected: String,
        /// What the on-disk bytes actually hash to.
        actual: String,
    },
    /// Fetched content did not match the expected address.
    #[error("verify: expected {expected}, got {actual}")]
    Mismatch {
        /// The expected content address.
        expected: String,
        /// The actual content address.
        actual: String,
    },
    /// A malformed content address.
    #[error("invalid hash: {0}")]
    InvalidHash(String),
    /// A malformed owner id.
    #[error("invalid owner: {0}")]
    InvalidOwner(String),
}

type Result<T> = std::result::Result<T, StoreError>;

/// A sha256 content address (64 lowercase hex chars).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash(String);

impl ContentHash {
    /// Parse and validate a 64-hex content address (case-insensitive, stored
    /// lowercase).
    pub fn parse(s: &str) -> Result<ContentHash> {
        if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(ContentHash(s.to_ascii_lowercase()))
        } else {
            Err(StoreError::InvalidHash(s.to_string()))
        }
    }

    /// Compute the content address of a byte slice.
    pub fn of(bytes: &[u8]) -> ContentHash {
        ContentHash(hex(&Sha256::digest(bytes)))
    }

    /// The hex string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a [`Store::gc`] pass removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// The objects that were collected.
    pub removed: Vec<ContentHash>,
    /// Total bytes freed.
    pub bytes_freed: u64,
}

/// Held for the duration of a mutating operation; releases the interprocess
/// lock on drop (flock is released when the file handle closes).
struct LockGuard {
    _file: fs::File,
}

/// A content-addressed, refcounted store rooted at a directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Store {
    /// Open (creating if needed) a store rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let root = root.into();
        // Create the skeleton durably: each newly created directory (including
        // a previously absent store root and any of its ancestors) has its
        // entry fsynced into its parent, so a crash right after first init
        // cannot lose the store or its objects/refs subdirectories.
        create_dir_durable(&root)?;
        create_dir_durable(&root.join("objects"))?;
        create_dir_durable(&root.join("refs"))?;
        Ok(Store { root })
    }

    /// Store `bytes` under their content address, returning it. Idempotent: an
    /// existing matching object is reused; an existing object whose bytes no
    /// longer match (corruption) is replaced.
    ///
    /// This stores but does not root the object. A bare `put` followed by a
    /// separate [`add_ref`] is **not atomic**: a concurrent [`gc`] can collect
    /// the still-unrooted object between the two calls, after which `add_ref`
    /// returns [`StoreError::NotFound`]. Fetch and install paths that must keep
    /// the object should use [`put_referenced`] or [`put_verified_referenced`],
    /// which store and root in one lock hold.
    ///
    /// [`add_ref`]: Store::add_ref
    /// [`gc`]: Store::gc
    /// [`put_referenced`]: Store::put_referenced
    /// [`put_verified_referenced`]: Store::put_verified_referenced
    pub fn put(&self, bytes: &[u8]) -> Result<ContentHash> {
        self.put_inner(bytes, None, None)
    }

    /// Store `bytes` only if they hash to `expected`, returning the verified
    /// address. Fails closed on a mismatch, storing nothing. Stores but does
    /// not root; see [`put_verified_referenced`] for the atomic verified
    /// store-and-root the fetch phase should use.
    ///
    /// [`put_verified_referenced`]: Store::put_verified_referenced
    pub fn put_verified(&self, bytes: &[u8], expected: &ContentHash) -> Result<ContentHash> {
        self.put_inner(bytes, Some(expected), None)
    }

    /// Store `bytes` and atomically root them to `owner` in one lock hold, so
    /// the object is never collectable in the window before it is referenced.
    /// Use this for trusted bytes (e.g. a freshly built artifact).
    pub fn put_referenced(&self, bytes: &[u8], owner: &str) -> Result<ContentHash> {
        self.put_inner(bytes, None, Some(owner))
    }

    /// Verify `bytes` against `expected` and, on a match, store and atomically
    /// root them to `owner` in one lock hold. This is the fetch phase's
    /// primitive: source verified against a recipe's declared `sha256` is
    /// rooted to the build before any GC can observe it. Fails closed on a
    /// mismatch, storing and rooting nothing.
    pub fn put_verified_referenced(
        &self,
        bytes: &[u8],
        expected: &ContentHash,
        owner: &str,
    ) -> Result<ContentHash> {
        self.put_inner(bytes, Some(expected), Some(owner))
    }

    /// Shared store path: verify the expected hash and validate the owner
    /// *before* taking the lock or mutating anything, then write (and optionally
    /// root) under a single lock hold so a verified-and-rooted object is never
    /// observable unrooted.
    fn put_inner(
        &self,
        bytes: &[u8],
        expected: Option<&ContentHash>,
        owner: Option<&str>,
    ) -> Result<ContentHash> {
        let actual = ContentHash::of(bytes);
        if let Some(expected) = expected {
            if &actual != expected {
                return Err(StoreError::Mismatch {
                    expected: expected.0.clone(),
                    actual: actual.0,
                });
            }
        }
        if let Some(owner) = owner {
            validate_owner(owner)?;
        }
        let _guard = self.lock()?;
        self.write_object_unlocked(&actual, bytes)?;
        if let Some(owner) = owner {
            self.add_ref_unlocked(&actual, owner)?;
        }
        Ok(actual)
    }

    /// Whether an object is present.
    pub fn has(&self, hash: &ContentHash) -> bool {
        self.object_path(hash).exists()
    }

    /// The on-disk path of an object, if present.
    pub fn object_path_if_present(&self, hash: &ContentHash) -> Option<PathBuf> {
        let p = self.object_path(hash);
        p.exists().then_some(p)
    }

    /// Read an object, verifying its on-disk bytes still match its address.
    pub fn read(&self, hash: &ContentHash) -> Result<Vec<u8>> {
        let path = self.object_path(hash);
        if !path.exists() {
            return Err(StoreError::NotFound(hash.0.clone()));
        }
        let bytes = fs::read(&path)?;
        let actual = ContentHash::of(&bytes);
        if &actual != hash {
            return Err(StoreError::Integrity {
                expected: hash.0.clone(),
                actual: actual.0,
            });
        }
        Ok(bytes)
    }

    /// Record that `owner` references `hash`. Idempotent.
    ///
    /// Returns [`StoreError::NotFound`] if the object is absent. Note the bare
    /// [`put`] + `add_ref` sequence is not atomic against a concurrent [`gc`]
    /// (see [`put`]); a fetch/install path should prefer [`put_referenced`] or
    /// [`put_verified_referenced`]. A caller using the bare sequence must handle
    /// `NotFound` by re-storing or retrying the object.
    ///
    /// [`put`]: Store::put
    /// [`gc`]: Store::gc
    /// [`put_referenced`]: Store::put_referenced
    /// [`put_verified_referenced`]: Store::put_verified_referenced
    pub fn add_ref(&self, hash: &ContentHash, owner: &str) -> Result<()> {
        validate_owner(owner)?;
        let _guard = self.lock()?;
        self.add_ref_unlocked(hash, owner)
    }

    /// Number of distinct owners referencing `hash`. Fails closed: an
    /// enumeration error is propagated rather than reported as zero, so a GC
    /// built on this never deletes an object whose refs cannot be read.
    pub fn refcount(&self, hash: &ContentHash) -> Result<usize> {
        let _guard = self.lock()?;
        self.refcount_unlocked(hash)
    }

    /// Drop every reference held by `owner` (e.g. on uninstall). Returns the
    /// number of references removed. Does not delete objects; call [`gc`] for
    /// that.
    ///
    /// [`gc`]: Store::gc
    pub fn release(&self, owner: &str) -> Result<usize> {
        validate_owner(owner)?;
        let _guard = self.lock()?;
        let mut removed = 0;
        let refs_root = self.root.join("refs");
        let entries = match fs::read_dir(&refs_root) {
            Ok(e) => e,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let dir = entry?.path();
            let marker = dir.join(owner);
            if marker.exists() {
                fs::remove_file(&marker)?;
                fsync_dir(&dir)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Remove every object no owner references, returning what was collected.
    /// Fails closed: if any object's refs cannot be enumerated, the whole pass
    /// errors rather than risk deleting referenced content.
    pub fn gc(&self) -> Result<GcReport> {
        let _guard = self.lock()?;
        let mut report = GcReport::default();
        let objects_root = self.root.join("objects");
        let shards = match fs::read_dir(&objects_root) {
            Ok(e) => e,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(e) => return Err(e.into()),
        };
        for shard in shards {
            let shard = shard?.path();
            if !shard.is_dir() {
                continue;
            }
            for object in fs::read_dir(&shard)? {
                let object = object?;
                let name = object.file_name();
                let Some(name) = name.to_str() else { continue };
                let Ok(hash) = ContentHash::parse(name) else {
                    continue;
                };
                // Fails closed: a refs-enumeration error aborts the pass.
                if self.refcount_unlocked(&hash)? == 0 {
                    let size = object.metadata().map(|m| m.len()).unwrap_or(0);
                    fs::remove_file(object.path())?;
                    let _ = fs::remove_dir(self.refs_dir(&hash));
                    report.bytes_freed += size;
                    report.removed.push(hash);
                }
            }
            fsync_dir(&shard)?;
        }
        report.removed.sort();
        Ok(report)
    }

    // -- internals (assume the store lock is held) ------------------------

    fn lock(&self) -> Result<LockGuard> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("lock"))?;
        file.lock_exclusive()?;
        Ok(LockGuard { _file: file })
    }

    fn add_ref_unlocked(&self, hash: &ContentHash, owner: &str) -> Result<()> {
        if !self.has(hash) {
            return Err(StoreError::NotFound(hash.0.clone()));
        }
        let dir = self.refs_dir(hash);
        let created = !dir.exists();
        fs::create_dir_all(&dir)?;
        // Make the new refs/<hash> entry durable in refs/ before the marker.
        if created {
            fsync_dir(&self.root.join("refs"))?;
        }
        fs::File::create(dir.join(owner))?.sync_all()?;
        fsync_dir(&dir)?;
        Ok(())
    }

    fn refcount_unlocked(&self, hash: &ContentHash) -> Result<usize> {
        let dir = self.refs_dir(hash);
        match fs::read_dir(&dir) {
            Ok(entries) => {
                let mut n = 0;
                for e in entries {
                    e?; // propagate a per-entry error rather than undercounting
                    n += 1;
                }
                Ok(n)
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    fn object_path(&self, hash: &ContentHash) -> PathBuf {
        self.root.join("objects").join(&hash.0[..2]).join(&hash.0)
    }

    fn refs_dir(&self, hash: &ContentHash) -> PathBuf {
        self.root.join("refs").join(&hash.0)
    }

    /// Place an object atomically. If it already exists, its bytes are verified
    /// and a corrupt object is repaired; a matching object is left untouched.
    fn write_object_unlocked(&self, hash: &ContentHash, bytes: &[u8]) -> Result<()> {
        let final_path = self.object_path(hash);
        if final_path.exists() {
            // Reuse only if the existing object is intact; otherwise repair it.
            match fs::read(&final_path) {
                Ok(existing) if &ContentHash::of(&existing) == hash => return Ok(()),
                _ => { /* fall through and overwrite with the verified bytes */ }
            }
        }
        let shard = final_path.parent().expect("object path has a shard parent");
        let shard_created = !shard.exists();
        fs::create_dir_all(shard)?;
        // Make the new objects/<ab> shard entry durable in objects/ before
        // publishing content into it.
        if shard_created {
            fsync_dir(&self.root.join("objects"))?;
        }
        // A per-call-unique temp file created with O_EXCL: never shared between
        // writers, even within one process.
        let tmp = shard.join(format!(
            ".tmp-{}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
            &hash.0[..8],
        ));
        {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        if let Err(e) = fs::rename(&tmp, &final_path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        fsync_dir(shard)?;
        Ok(())
    }
}

/// Create `path` (and any missing ancestors) durably: each newly created
/// directory has its entry fsynced into its parent, so the new directory
/// survives a crash immediately after creation. A no-op if `path` exists.
///
/// Limitation: this is lock-free, so two processes performing the *first ever*
/// initialization of the same store concurrently can race — one may observe a
/// directory the other created but has not yet parent-fsynced. A crash in that
/// sub-millisecond window can lose the entry. This self-heals: `open` is
/// idempotent and recreates any missing skeleton on the next run, and any lost
/// content is content-addressed and re-fetched on demand, so the store
/// converges to a consistent (possibly emptier) state with no corruption or
/// dangling refs. The realistic deployment creates the store root once via the
/// installer before forage runs.
fn create_dir_durable(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    // The chain of ancestors that do not yet exist, deepest first.
    let mut missing = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        cur = p.parent();
    }
    fs::create_dir_all(path)?;
    // Fsync each new directory's parent, shallowest first, so a parent's own
    // entry is durable before the child's is published into it. A relative
    // top-level component has an empty parent, which means the current
    // directory: fsync `.` rather than silently skipping it.
    for p in missing.iter().rev() {
        match p.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => fsync_dir(parent)?,
            Some(_) => fsync_dir(std::path::Path::new("."))?,
            None => {} // `p` is the filesystem root; nothing above it to fsync
        }
    }
    Ok(())
}

/// fsync a directory so a rename/create/remove within it survives a crash.
/// A missing directory is not an error (nothing to make durable).
fn fsync_dir(dir: &std::path::Path) -> Result<()> {
    match fs::File::open(dir) {
        Ok(f) => {
            f.sync_all()?;
            Ok(())
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn validate_owner(owner: &str) -> Result<()> {
    let bad = owner.is_empty()
        || owner == "."
        || owner == ".."
        || owner.contains('/')
        || owner.contains('\\')
        || owner.contains('\0');
    if bad {
        Err(StoreError::InvalidOwner(owner.to_string()))
    } else {
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn put_is_content_addressed_and_dedups() {
        let (_d, s) = store();
        let h1 = s.put(b"hello world").unwrap();
        let h2 = s.put(b"hello world").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(
            h1.as_str(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert_eq!(s.read(&h1).unwrap(), b"hello world");
    }

    #[test]
    fn read_detects_corruption() {
        let (_d, s) = store();
        let h = s.put(b"trustworthy").unwrap();
        fs::write(s.object_path(&h), b"tampered!!!").unwrap();
        assert!(matches!(s.read(&h).unwrap_err(), StoreError::Integrity { .. }));
    }

    #[test]
    fn put_repairs_a_corrupt_existing_object() {
        let (_d, s) = store();
        let h = s.put(b"good bytes").unwrap();
        fs::write(s.object_path(&h), b"corrupt!!!").unwrap();
        // Re-putting the correct bytes overwrites the corruption.
        let h2 = s.put(b"good bytes").unwrap();
        assert_eq!(h, h2);
        assert_eq!(s.read(&h).unwrap(), b"good bytes");
    }

    #[test]
    fn put_verified_fails_closed_on_mismatch() {
        let (_d, s) = store();
        let wrong = ContentHash::of(b"something else");
        assert!(matches!(
            s.put_verified(b"the real bytes", &wrong).unwrap_err(),
            StoreError::Mismatch { .. }
        ));
        assert!(!s.has(&ContentHash::of(b"the real bytes")));
    }

    #[test]
    fn put_referenced_is_atomic_store_and_root() {
        let (_d, s) = store();
        let h = s.put_referenced(b"source tree", "org.example.app").unwrap();
        assert_eq!(s.refcount(&h).unwrap(), 1);
        // GC cannot collect it: it was rooted in the same lock hold.
        assert_eq!(s.gc().unwrap().removed, vec![]);
        assert!(s.has(&h));
    }

    #[test]
    fn open_creates_a_nested_absent_root() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c/store");
        assert!(!nested.exists());
        let s = Store::open(&nested).unwrap();
        let h = s.put(b"works").unwrap();
        assert_eq!(s.read(&h).unwrap(), b"works");
        assert!(nested.join("objects").is_dir());
        assert!(nested.join("refs").is_dir());
    }

    #[test]
    fn put_verified_referenced_verifies_and_roots_atomically() {
        let (_d, s) = store();
        let expected = ContentHash::of(b"recipe source");
        // Mismatch: verified before any mutation, stores and roots nothing.
        let wrong = ContentHash::of(b"other");
        assert!(matches!(
            s.put_verified_referenced(b"recipe source", &wrong, "org.example.app"),
            Err(StoreError::Mismatch { .. })
        ));
        assert!(!s.has(&expected));
        // Match: stored and rooted, survives gc.
        let h = s
            .put_verified_referenced(b"recipe source", &expected, "org.example.app")
            .unwrap();
        assert_eq!(h, expected);
        assert_eq!(s.refcount(&h).unwrap(), 1);
        assert_eq!(s.gc().unwrap().removed, vec![]);
    }

    #[test]
    fn refcount_tracks_distinct_owners() {
        let (_d, s) = store();
        let h = s.put(b"shared lib").unwrap();
        assert_eq!(s.refcount(&h).unwrap(), 0);
        s.add_ref(&h, "org.example.app").unwrap();
        s.add_ref(&h, "org.example.app").unwrap();
        s.add_ref(&h, "org.example.other").unwrap();
        assert_eq!(s.refcount(&h).unwrap(), 2);
    }

    #[test]
    fn gc_collects_only_unreferenced() {
        let (_d, s) = store();
        let kept = s.put(b"still used").unwrap();
        let dropped = s.put(b"orphan").unwrap();
        s.add_ref(&kept, "org.example.app").unwrap();
        let report = s.gc().unwrap();
        assert_eq!(report.removed, vec![dropped.clone()]);
        assert!(s.has(&kept));
        assert!(!s.has(&dropped));
    }

    #[test]
    fn release_then_gc_collects() {
        let (_d, s) = store();
        let h = s.put(b"lib").unwrap();
        s.add_ref(&h, "org.example.app").unwrap();
        assert_eq!(s.gc().unwrap().removed, vec![]);
        assert_eq!(s.release("org.example.app").unwrap(), 1);
        assert_eq!(s.refcount(&h).unwrap(), 0);
        assert_eq!(s.gc().unwrap().removed, vec![h.clone()]);
    }

    #[test]
    fn add_ref_to_absent_object_fails() {
        let (_d, s) = store();
        let absent = ContentHash::of(b"never stored");
        assert!(matches!(
            s.add_ref(&absent, "org.example.app"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn rejects_bad_hash_and_owner() {
        assert!(ContentHash::parse("xyz").is_err());
        assert!(ContentHash::parse(&"a".repeat(63)).is_err());
        assert!(ContentHash::parse(&"A".repeat(64)).is_ok());
        let (_d, s) = store();
        let h = s.put(b"x").unwrap();
        for bad in ["", "..", "a/b", "a\\b"] {
            assert!(matches!(s.add_ref(&h, bad), Err(StoreError::InvalidOwner(_))));
        }
    }

    #[test]
    fn concurrent_put_referenced_always_survives_gc() {
        // put_referenced stores and roots in one lock hold, so a racing GC
        // (which needs the same lock) runs strictly before or after it and can
        // never collect the object.
        for round in 0..50u32 {
            let (_d, s) = store();
            let s = Arc::new(s);
            let payload = format!("payload-{round}");
            let s1 = Arc::clone(&s);
            let adder = std::thread::spawn(move || {
                s1.put_referenced(payload.as_bytes(), "org.example.app").unwrap()
            });
            let s2 = Arc::clone(&s);
            let gcer = std::thread::spawn(move || s2.gc().unwrap());
            let h = adder.join().unwrap();
            gcer.join().unwrap();
            assert_eq!(s.refcount(&h).unwrap(), 1, "rooted object keeps its ref");
            assert!(s.has(&h), "atomically-rooted object survives gc");
        }
    }

    #[test]
    fn concurrent_bare_put_then_addref_vs_gc_keeps_the_invariant() {
        // A bare put followed by a separate add_ref is deliberately NOT atomic:
        // a GC between them may collect the still-unrooted object, in which case
        // add_ref correctly returns NotFound. The invariant that must always
        // hold either way: no ref marker ever survives without its object.
        for round in 0..50u32 {
            let (_d, s) = store();
            let s = Arc::new(s);
            let h = s.put(format!("p-{round}").as_bytes()).unwrap();
            let s1 = Arc::clone(&s);
            let h1 = h.clone();
            let adder = std::thread::spawn(move || s1.add_ref(&h1, "org.example.app"));
            let s2 = Arc::clone(&s);
            let gcer = std::thread::spawn(move || s2.gc().unwrap());
            let added = adder.join().unwrap();
            gcer.join().unwrap();
            match added {
                // add_ref committed under the lock, so it ran before gc and the
                // object is kept (gc then saw the ref).
                Ok(()) => {
                    assert!(s.has(&h));
                    assert_eq!(s.refcount(&h).unwrap(), 1);
                }
                // gc won the lock first and collected the unrooted object; no
                // dangling ref is left behind.
                Err(StoreError::NotFound(_)) => {
                    assert!(!s.has(&h));
                    assert_eq!(s.refcount(&h).unwrap(), 0);
                }
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
    }

    #[test]
    fn concurrent_same_object_writers_publish_intact_content() {
        let (_d, s) = store();
        let s = Arc::new(s);
        let payload = vec![7u8; 4096];
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = Arc::clone(&s);
            let p = payload.clone();
            handles.push(std::thread::spawn(move || s.put(&p).unwrap()));
        }
        let hashes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(hashes.iter().all(|h| *h == hashes[0]));
        // The published object is intact (read verifies the address).
        assert_eq!(s.read(&hashes[0]).unwrap(), payload);
    }

    #[test]
    fn content_hash_normalizes_case_and_round_trips() {
        let h = ContentHash::of(b"data");
        // `of` yields 64 lowercase hex.
        assert_eq!(h.as_str().len(), 64);
        assert!(h.as_str().bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        // Parsing its own string round-trips.
        assert_eq!(ContentHash::parse(h.as_str()).unwrap(), h);
        // The uppercase spelling is the same address (parse lowercases).
        assert_eq!(ContentHash::parse(&h.as_str().to_ascii_uppercase()).unwrap(), h);
        // 64 non-hex chars and a 65-char hex string are both rejected.
        assert!(ContentHash::parse(&"g".repeat(64)).is_err());
        assert!(ContentHash::parse(&"a".repeat(65)).is_err());
    }

    #[test]
    fn validate_owner_rejects_dot_and_nul_accepts_reverse_dns() {
        let (_d, s) = store();
        let h = s.put(b"obj").unwrap();
        // The single-dot and embedded-NUL branches (not hit by the existing test).
        for bad in [".", "x\0y"] {
            assert!(
                matches!(s.add_ref(&h, bad), Err(StoreError::InvalidOwner(_))),
                "{bad:?} must be an invalid owner"
            );
        }
        // A reverse-DNS owner with hyphen/underscore/dot is accepted.
        assert!(s.add_ref(&h, "org.example.app-v2_beta").is_ok());
        assert_eq!(s.refcount(&h).unwrap(), 1);
    }

    #[test]
    fn release_of_an_unreferenced_owner_returns_zero() {
        let (_d, s) = store();
        let h = s.put(b"obj").unwrap();
        s.add_ref(&h, "org.example.app").unwrap();
        // Releasing an owner that holds no refs removes nothing.
        assert_eq!(s.release("org.example.never").unwrap(), 0);
        assert_eq!(s.refcount(&h).unwrap(), 1);
    }
}
