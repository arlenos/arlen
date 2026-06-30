//! The model-download store: stream a byte source to disk, verify its sha256,
//! and store it atomically. The network fetch (SSRF-pinned through the egress)
//! feeds [`verify_and_store`] a byte stream; the EXPECTED sha256 (a catalog pin
//! or Hugging Face file metadata) is the caller's concern - kept out of this
//! integrity core so it is unit-testable without a network. Streaming (a 1 MiB
//! buffer) so a multi-GiB model never loads into memory.

use std::io::{self, Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

/// A sha256 digest is 64 lowercase hex characters.
const SHA256_HEX_LEN: usize = 64;
/// Streaming copy buffer; models are GiB-scale, so never buffer the whole file.
const COPY_BUF: usize = 1 << 20;

/// Why storing a downloaded model failed.
#[derive(Debug)]
pub enum StoreError {
    /// An I/O error reading the stream or writing the store.
    Io(io::Error),
    /// The expected sha256 is not 64 hex characters.
    BadShaHex,
    /// The downloaded content did not hash to the expected sha256 (the file is
    /// discarded; the destination is untouched).
    ShaMismatch {
        /// The sha256 the caller pinned.
        expected: String,
        /// The sha256 the downloaded bytes actually hashed to.
        got: String,
    },
    /// The destination path has no parent directory to stage the temp file in.
    NoParent,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "model store io error: {e}"),
            StoreError::BadShaHex => write!(f, "expected sha256 is not 64 hex characters"),
            StoreError::ShaMismatch { expected, got } => {
                write!(f, "sha256 mismatch: expected {expected}, got {got}")
            }
            StoreError::NoParent => write!(f, "destination has no parent directory"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Stream `reader` into `dest`, verifying the content hashes to
/// `expected_sha256` (lowercase hex, 64 chars). The bytes are written to a temp
/// file in `dest`'s own directory while hashing; on a sha MATCH the temp file is
/// fsynced and atomically renamed onto `dest`; on a mismatch (or any error) the
/// temp file is discarded and `dest` is left untouched - so a corrupt or
/// truncated download can never leave a usable model in place. The temp lives in
/// the destination directory so the rename is atomic on the same filesystem.
pub fn verify_and_store<R: Read>(
    mut reader: R,
    expected_sha256: &str,
    dest: &Path,
) -> Result<(), StoreError> {
    let expected = expected_sha256.trim().to_ascii_lowercase();
    if expected.len() != SHA256_HEX_LEN || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StoreError::BadShaHex);
    }
    let parent = dest.parent().ok_or(StoreError::NoParent)?;
    std::fs::create_dir_all(parent).map_err(StoreError::Io)?;

    let mut tmp = NamedTempFile::new_in(parent).map_err(StoreError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_BUF];
    loop {
        let n = reader.read(&mut buf).map_err(StoreError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        tmp.write_all(&buf[..n]).map_err(StoreError::Io)?;
    }

    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != expected {
        // `tmp` is dropped here, removing the partial file; `dest` is untouched.
        return Err(StoreError::ShaMismatch { expected, got });
    }

    tmp.as_file().sync_all().map_err(StoreError::Io)?;
    // Atomic rename into place; only a sha-verified file ever reaches `dest`.
    tmp.persist(dest).map_err(|e| StoreError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha_hex(data: &[u8]) -> String {
        Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn stores_on_match_creating_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("models/sub/model.gguf"); // parent does not exist yet
        let data = b"pretend model weights";
        verify_and_store(&data[..], &sha_hex(data), &dest).expect("stores on match");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[test]
    fn rejects_mismatch_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let data = b"pretend model weights";
        let wrong = "0".repeat(64);
        let err = verify_and_store(&data[..], &wrong, &dest).unwrap_err();
        assert!(matches!(err, StoreError::ShaMismatch { .. }));
        assert!(!dest.exists(), "a sha-mismatched download must not be stored");
        // The destination directory holds no leftover temp files either.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(leftovers.is_empty(), "the discarded temp file is cleaned up");
    }

    #[test]
    fn rejects_malformed_expected_sha() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        assert!(matches!(
            verify_and_store(&b"x"[..], "not-hex", &dest),
            Err(StoreError::BadShaHex)
        ));
        assert!(matches!(
            verify_and_store(&b"x"[..], &"a".repeat(63), &dest), // wrong length
            Err(StoreError::BadShaHex)
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn accepts_uppercase_expected_sha() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let data = b"weights";
        verify_and_store(&data[..], &sha_hex(data).to_ascii_uppercase(), &dest)
            .expect("uppercase hex normalizes");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }
}
