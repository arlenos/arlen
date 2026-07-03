//! Chunked download copy with progress reporting + cancellation.
//!
//! The consent-gated model downloader needs three things the blocking
//! [`crate::fetch::download_model`] does not surface: live progress (bytes so
//! far / total), mid-transfer cancellation, and the content sha256 as the bytes
//! land. This is that mechanism, kept PURE over a reader and a writer so it never
//! touches the SSRF-safe egress layer: a real download plugs its pinned, redirect-
//! validated response body in as the `reader`, and the consent/egress gate is
//! enforced ABOVE this copy (this only moves already-authorised bytes).

use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// Copy chunk size. 64 KiB balances syscall overhead against a responsive
/// progress + cancel cadence (a check + callback per chunk).
const CHUNK: usize = 64 * 1024;

/// A cancellable, progress-reporting streaming copy failed.
#[derive(Debug)]
pub enum StreamError {
    /// A read from the source or a write to the destination failed.
    Io(std::io::Error),
    /// `is_cancelled` returned true between chunks; the partial destination is
    /// the caller's to discard (a downloader unlinks the temp file).
    Cancelled,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Io(e) => write!(f, "stream io error: {e}"),
            StreamError::Cancelled => write!(f, "download cancelled"),
        }
    }
}

impl std::error::Error for StreamError {}

/// The result of a completed streaming copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamOutcome {
    /// Total bytes copied.
    pub bytes: u64,
    /// Lowercase-hex sha256 of the copied bytes (for the caller to compare
    /// against the catalogue's expected digest).
    pub sha256_hex: String,
}

/// Copy `reader` to `writer` in [`CHUNK`]-sized pieces, hashing as it goes and
/// reporting `on_progress(bytes_so_far, total)` after each chunk. Before each read
/// it consults `is_cancelled()`; a `true` aborts with [`StreamError::Cancelled`].
/// `total` is the expected length for the progress fraction (`0` = unknown, e.g. no
/// Content-Length). Returns the byte count + the content sha256 on completion.
pub fn stream_to_writer<R, W, P, C>(
    mut reader: R,
    mut writer: W,
    total: u64,
    mut on_progress: P,
    is_cancelled: C,
) -> Result<StreamOutcome, StreamError>
where
    R: Read,
    W: Write,
    P: FnMut(u64, u64),
    C: Fn() -> bool,
{
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut bytes: u64 = 0;
    loop {
        if is_cancelled() {
            return Err(StreamError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(StreamError::Io)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).map_err(StreamError::Io)?;
        hasher.update(&buf[..n]);
        bytes += n as u64;
        on_progress(bytes, total);
    }
    writer.flush().map_err(StreamError::Io)?;
    Ok(StreamOutcome {
        bytes,
        sha256_hex: to_hex(&hasher.finalize()),
    })
}

/// Lowercase-hex encode a byte slice.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The empty-input sha256 (a known vector), so the hash path is verified.
    const SHA256_EMPTY: &str =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn copies_bytes_reports_progress_and_hashes() {
        let data = b"hello arlen streaming download".to_vec();
        let mut dest = Vec::new();
        let mut progress = Vec::new();
        let outcome = stream_to_writer(
            Cursor::new(data.clone()),
            &mut dest,
            data.len() as u64,
            |so_far, total| progress.push((so_far, total)),
            || false,
        )
        .expect("copy ok");
        assert_eq!(dest, data, "destination is a faithful copy");
        assert_eq!(outcome.bytes, data.len() as u64);
        // Progress ends at the full length.
        assert_eq!(progress.last(), Some(&(data.len() as u64, data.len() as u64)));
        // sha256 matches an independent computation.
        let expected = to_hex(&Sha256::digest(&data));
        assert_eq!(outcome.sha256_hex, expected);
    }

    #[test]
    fn empty_input_yields_the_empty_sha256() {
        let mut dest = Vec::new();
        let outcome =
            stream_to_writer(Cursor::new(Vec::new()), &mut dest, 0, |_, _| {}, || false).unwrap();
        assert_eq!(outcome.bytes, 0);
        assert_eq!(outcome.sha256_hex, SHA256_EMPTY);
    }

    #[test]
    fn a_set_cancel_flag_aborts_before_any_read() {
        // A payload larger than one chunk so a cancel is meaningful.
        let data = vec![7u8; CHUNK * 3];
        let mut dest = Vec::new();
        let cancelled = AtomicBool::new(true);
        let err = stream_to_writer(
            Cursor::new(data),
            &mut dest,
            0,
            |_, _| {},
            || cancelled.load(Ordering::Relaxed),
        )
        .expect_err("cancelled");
        assert!(matches!(err, StreamError::Cancelled));
        assert!(dest.is_empty(), "nothing copied when cancelled up front");
    }

    #[test]
    fn cancel_mid_stream_stops_the_copy() {
        let data = vec![3u8; CHUNK * 4];
        let mut dest = Vec::new();
        let cancelled = AtomicBool::new(false);
        let err = stream_to_writer(
            Cursor::new(data),
            &mut dest,
            0,
            // Trip the cancel once the first chunk has been written, so the next
            // loop iteration's pre-read check aborts.
            |_, _| cancelled.store(true, Ordering::Relaxed),
            || cancelled.load(Ordering::Relaxed),
        )
        .expect_err("cancelled mid-stream");
        assert!(matches!(err, StreamError::Cancelled));
        // Exactly the first chunk landed before the cancel took effect.
        assert_eq!(dest.len(), CHUNK);
    }
}
