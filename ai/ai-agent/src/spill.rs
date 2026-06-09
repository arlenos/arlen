//! Tool-result spill store (B-spill): persist a full observation result by its
//! content address so the bounded inline preview can be dropped during
//! compaction without losing the result.
//!
//! The agent's working memory keeps only a short, inert, screened preview of a
//! read result plus its `result_ref`; the full rows are large and do not belong
//! in the prompt. This store holds those full rows, fetchable by the same ref,
//! so the deferred LLM-summary compaction tier (B-compact) can summarise a
//! result the preview already dropped, and any consumer can re-expand one.
//!
//! It is content-addressed and write-once: the ref IS the `sha256:<hex>` of the
//! stored bytes (the scheme [`content_ref`] defines and the observation builder
//! reuses), so a read-back verifies the bytes re-hash to the ref (a corrupt file
//! reads as absent, fail-closed) and re-spilling identical content is a no-op.

use std::io;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// A content-addressed store for full tool results.
pub trait SpillStore: Send + Sync {
    /// Persist `bytes` under content-address `reference` (the `sha256:<hex>`
    /// form). Write-once and idempotent. Best-effort by contract: the caller
    /// treats a failure as non-fatal (the inline preview still drives the
    /// prompt), so spilling never aborts the agent loop.
    fn put(&self, reference: &str, bytes: &[u8]) -> io::Result<()>;

    /// Fetch the full result previously spilled under `reference`, verifying it
    /// re-hashes to that reference. Returns `None` if absent, oversized-and-
    /// skipped, or corrupt.
    fn get(&self, reference: &str) -> io::Result<Option<Vec<u8>>>;
}

/// A filesystem spill store: one content-addressed file per result under a
/// directory (typically a per-run directory on tmpfs). The agent loop is serial,
/// so a single fixed temp name per write is sufficient for atomicity.
pub struct FileSpillStore {
    dir: PathBuf,
    max_bytes: usize,
}

impl FileSpillStore {
    /// A store under `dir`. A single result larger than `max_bytes` is not
    /// spilled (its ref stays an identity with no backing file), bounding any one
    /// file so a pathological result cannot fill the disk. Directory-level GC of
    /// old results is a follow-up; today a per-run dir bounds total lifetime.
    pub fn new(dir: impl Into<PathBuf>, max_bytes: usize) -> Self {
        Self {
            dir: dir.into(),
            max_bytes,
        }
    }

    /// The on-disk path for a reference, or `None` if the reference is not the
    /// exact `sha256:<64 lowercase hex>` shape. Validating the shape means the
    /// filename is a fixed-form component with no separators or traversal.
    fn path_for(&self, reference: &str) -> Option<PathBuf> {
        let hex = reference.strip_prefix("sha256:")?;
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return None;
        }
        Some(self.dir.join(hex))
    }
}

impl SpillStore for FileSpillStore {
    fn put(&self, reference: &str, bytes: &[u8]) -> io::Result<()> {
        // Skip an oversized result: the ref stays identity-only, which is the
        // pre-spill behaviour, and get() returns None for it.
        if bytes.len() > self.max_bytes {
            return Ok(());
        }
        let path = self
            .path_for(reference)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid spill reference"))?;
        // Write-once: identical content addressed by the same ref is already here.
        if path.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        // Atomic publish: write a temp file then rename it into place, so a
        // reader never sees a half-written result.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)
    }

    fn get(&self, reference: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(path) = self.path_for(reference) else {
            return Ok(None);
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                // Integrity: the stored bytes must re-hash to the reference, or
                // the file is corrupt and is treated as absent (fail-closed).
                if content_ref(&bytes) == reference {
                    Ok(Some(bytes))
                } else {
                    Ok(None)
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The `sha256:<hex>` content-address of `bytes`. This is the one hashing scheme
/// the spill store and the observation builder share, so a spilled result's ref
/// matches the `Observation`'s `result_ref` exactly.
pub fn content_ref(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_round_trips_by_content_address() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSpillStore::new(dir.path(), 1024);
        let bytes = b"[{\"file\":\"a.rs\"}]";
        let reference = content_ref(bytes);
        store.put(&reference, bytes).unwrap();
        assert_eq!(store.get(&reference).unwrap().as_deref(), Some(&bytes[..]));
    }

    #[test]
    fn an_absent_reference_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSpillStore::new(dir.path(), 1024);
        let reference = content_ref(b"never stored");
        assert!(store.get(&reference).unwrap().is_none());
    }

    #[test]
    fn an_oversized_result_is_not_spilled() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSpillStore::new(dir.path(), 8);
        let bytes = b"this is longer than eight bytes";
        let reference = content_ref(bytes);
        store.put(&reference, bytes).unwrap(); // no error, just skipped
        assert!(store.get(&reference).unwrap().is_none(), "oversized is not stored");
    }

    #[test]
    fn a_malformed_reference_is_rejected_on_put_and_none_on_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSpillStore::new(dir.path(), 1024);
        assert!(store.put("not-a-ref", b"x").is_err());
        assert!(store.put("sha256:tooshort", b"x").is_err());
        assert!(store.get("../escape").unwrap().is_none());
    }

    #[test]
    fn a_corrupt_file_reads_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSpillStore::new(dir.path(), 1024);
        let bytes = b"[{\"file\":\"a.rs\"}]";
        let reference = content_ref(bytes);
        store.put(&reference, bytes).unwrap();
        // Tamper the stored file: it no longer re-hashes to the reference.
        let hex = reference.strip_prefix("sha256:").unwrap();
        std::fs::write(dir.path().join(hex), b"tampered").unwrap();
        assert!(store.get(&reference).unwrap().is_none(), "corruption fails closed");
    }

    #[test]
    fn re_spilling_identical_content_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSpillStore::new(dir.path(), 1024);
        let bytes = b"[{\"k\":1}]";
        let reference = content_ref(bytes);
        store.put(&reference, bytes).unwrap();
        store.put(&reference, bytes).unwrap(); // write-once, no error
        assert_eq!(store.get(&reference).unwrap().as_deref(), Some(&bytes[..]));
    }
}
