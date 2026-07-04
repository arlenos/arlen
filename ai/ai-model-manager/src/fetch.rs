//! The consent-gated model download: fetch a GGUF over the SSRF-safe egress and
//! store it sha256-verified.
//!
//! This is the one explicit, user-consented outbound a no-telemetry OS makes
//! (`local-model-bundle-plan.md`). It mirrors the proven `arlen-forage-fetch`
//! egress discipline (resolve the host ourselves through net-guard's
//! [`resolve_and_pin`], reject blocked ranges, pin reqwest to the verified
//! address so DNS rebinding cannot swap the target, HTTPS only, no ambient
//! proxy), with one difference that matters for a GiB-scale model: the body is
//! never buffered. reqwest's redirect follower is disabled so we follow each
//! `Location` ourselves and RE-PIN every hop (Hugging Face `/resolve/main/`
//! 302s to a CDN host), and the final response - which implements [`io::Read`] -
//! is streamed straight into [`crate::store::verify_and_store`], which hashes
//! and atomically stores it in a single pass.
//!
//! `resolve_and_pin` is async; it runs on a one-shot current-thread runtime
//! whose `block_on` returns before the blocking GET, so the blocking client is
//! never started from inside an async context.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use arlen_net_guard::{resolve_and_pin, GuardError};

use crate::store::{verify_and_store, StoreError};

/// Observes a download as it streams: a progress callback and a cancel flag, both
/// supplied by the caller (a UI passes a closure that emits progress events and a
/// flag it flips to cancel). Threaded into [`download_model`] as an option, so a
/// headless/first-run download can pass `None`.
pub struct DownloadObserver<'a> {
    /// Called after each read with `(bytes_so_far, total_bytes)`; `total` is 0
    /// when the server sent no `Content-Length`.
    pub on_progress: &'a (dyn Fn(u64, u64) + Sync),
    /// Checked before each read; a set flag aborts with [`DownloadError::Cancelled`].
    pub cancel: &'a AtomicBool,
}

/// A `Read` that reports progress and honours cancellation as the body streams
/// through it into the sha-verified store, so a GiB body never buffers to observe it.
struct ProgressReader<'a, R> {
    inner: R,
    read: u64,
    total: u64,
    observer: &'a DownloadObserver<'a>,
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.observer.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "download cancelled",
            ));
        }
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.read += n as u64;
            (self.observer.on_progress)(self.read, self.total);
        }
        Ok(n)
    }
}

/// Maximum redirect hops followed before giving up. HF's resolve URL needs one
/// (origin -> CDN); the rest is slack. Each hop is independently SSRF-pinned.
const MAX_REDIRECT_HOPS: usize = 5;
/// Wall-clock bound on the whole transfer. A GGUF is hundreds of MB to a few GB,
/// so this is generous for a slow link while still killing a hung server.
const FETCH_TIMEOUT_SECS: u64 = 3600;
/// Hard cap on the streamed body, so a runaway or compromised CDN cannot fill
/// the disk inside the timeout window. Comfortably above the largest catalog
/// model (an 8-bit 32B GGUF is ~35 GB) while bounding the worst-case transient
/// write; a real file over this would truncate and fail the sha check (and be
/// discarded), which is the safe direction.
const MAX_MODEL_BYTES: u64 = 48 * 1024 * 1024 * 1024;

/// Why a model download failed.
#[derive(Debug)]
pub enum DownloadError {
    /// The url (or a redirect target) was not `https`.
    NotHttps(String),
    /// The url had no host component.
    NoHost(String),
    /// net-guard refused the destination (a blocked / private / rebinding IP).
    Blocked(String),
    /// A network/transport error (DNS, connect, TLS, body).
    Network(String),
    /// The redirect chain exceeded [`MAX_REDIRECT_HOPS`].
    TooManyRedirects,
    /// The server returned a non-success, non-redirect status.
    Status(u16),
    /// Storing or verifying the downloaded bytes failed (incl. a sha mismatch).
    Store(StoreError),
    /// The async resolver runtime could not be built.
    Runtime(String),
    /// The caller flipped the observer's cancel flag mid-transfer; the partial
    /// file is discarded by the store rather than left half-written.
    Cancelled,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::NotHttps(u) => write!(f, "model url is not https: {u}"),
            DownloadError::NoHost(u) => write!(f, "model url has no host: {u}"),
            DownloadError::Blocked(m) => write!(f, "destination blocked by the egress guard: {m}"),
            DownloadError::Network(m) => write!(f, "model download network error: {m}"),
            DownloadError::TooManyRedirects => {
                write!(f, "model download exceeded {MAX_REDIRECT_HOPS} redirects")
            }
            DownloadError::Status(s) => write!(f, "model download got HTTP status {s}"),
            DownloadError::Store(e) => write!(f, "{e}"),
            DownloadError::Runtime(m) => write!(f, "model download runtime error: {m}"),
            DownloadError::Cancelled => write!(f, "model download cancelled"),
        }
    }
}

impl std::error::Error for DownloadError {}

/// Download a GGUF from `url` to `dest`, verifying it hashes to
/// `expected_sha256` (the caller's pin: a catalog entry or HF file metadata).
/// SSRF-pinned and redirect-following with a per-hop re-pin; the body streams
/// straight into the verified store, never buffered. On any failure `dest` is
/// left untouched (the verified store discards a partial or mismatched file).
///
/// PRECONDITION: call this from a SYNCHRONOUS context, never from inside a tokio
/// runtime (an async task / `#[tokio::main]`). It drives an async resolver on
/// its own current-thread runtime and then a blocking HTTP GET, both of which
/// panic if started from within a runtime. A Tauri/daemon caller must wrap it in
/// `tokio::task::spawn_blocking` or a dedicated OS thread. The guard below turns
/// a violation into a clean error rather than a panic.
pub fn download_model(
    url: &str,
    expected_sha256: &str,
    dest: &Path,
    observer: Option<&DownloadObserver>,
) -> Result<(), DownloadError> {
    // Fail loud, not panic, if invoked from an async context (see PRECONDITION).
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(DownloadError::Runtime(
            "download_model must run in a synchronous context (use spawn_blocking), \
             not inside a tokio runtime"
                .into(),
        ));
    }

    // A dedicated current-thread runtime drives the async SSRF resolver. Each
    // `block_on` enters and exits before the blocking GET below, so reqwest's
    // blocking client is never created or used from within an async context.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| DownloadError::Runtime(e.to_string()))?;

    let mut current = url.to_string();
    // Initial request + up to MAX_REDIRECT_HOPS redirects.
    for _ in 0..=MAX_REDIRECT_HOPS {
        let parsed = reqwest::Url::parse(&current)
            .map_err(|e| DownloadError::Network(format!("parse url: {e}")))?;
        if parsed.scheme() != "https" {
            return Err(DownloadError::NotHttps(current));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| DownloadError::NoHost(current.clone()))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(443);

        // SSRF guard + DNS-rebinding pin for THIS hop's host.
        let addr = match rt.block_on(resolve_and_pin(&host, port)) {
            Ok(addr) => addr,
            Err(blocked @ GuardError::Blocked { .. }) => {
                return Err(DownloadError::Blocked(blocked.to_string()))
            }
            Err(e) => return Err(DownloadError::Network(e.to_string())),
        };

        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            // No ambient proxy may sit in front of the origin we pinned.
            .no_proxy()
            // Follow redirects ourselves so every hop is re-pinned.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
            .user_agent(concat!("Arlen-model-manager/", env!("CARGO_PKG_VERSION")))
            .resolve(&host, addr)
            .build()
            .map_err(|e| DownloadError::Network(format!("client: {e}")))?;

        let resp = client
            .get(&current)
            .send()
            .map_err(|e| DownloadError::Network(e.to_string()))?;
        let status = resp.status();

        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| DownloadError::Network("redirect without a Location".into()))?;
            // Resolve a relative redirect against the current url; an absolute
            // one replaces it. The next loop re-pins the (possibly new) host.
            current = parsed
                .join(location)
                .map_err(|e| DownloadError::Network(format!("redirect target: {e}")))?
                .to_string();
            continue;
        }

        if !status.is_success() {
            return Err(DownloadError::Status(status.as_u16()));
        }

        // Final 200: the blocking response is a `Read`, streamed straight into
        // the sha-verified atomic store. A GiB body never lands in memory, and
        // `take` bounds the bytes written so a runaway server cannot fill the
        // disk (a real file over the cap truncates and fails the sha check).
        let total = resp.content_length().unwrap_or(0);
        let body = resp.take(MAX_MODEL_BYTES);
        return match observer {
            None => verify_and_store(body, expected_sha256, dest).map_err(DownloadError::Store),
            Some(obs) => {
                let reader = ProgressReader {
                    inner: body,
                    read: 0,
                    total,
                    observer: obs,
                };
                let result = verify_and_store(reader, expected_sha256, dest);
                // A cancel surfaces as an Interrupted io error inside the store;
                // report it as Cancelled, not a generic store failure.
                if result.is_err() && obs.cancel.load(Ordering::Relaxed) {
                    return Err(DownloadError::Cancelled);
                }
                result.map_err(DownloadError::Store)
            }
        };
    }

    Err(DownloadError::TooManyRedirects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_url() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        let err = download_model("http://example.com/m.gguf", &"a".repeat(64), &dest, None).unwrap_err();
        assert!(matches!(err, DownloadError::NotHttps(_)));
        assert!(!dest.exists());
    }

    #[test]
    fn refuses_to_run_inside_a_tokio_runtime() {
        // Called from within a runtime, it must return a clean error, not panic
        // (the precondition guard), so a future async caller fails loud.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        let err = rt.block_on(async {
            download_model("https://example.com/m.gguf", &"a".repeat(64), &dest, None)
        });
        assert!(matches!(err, Err(DownloadError::Runtime(_))));
        assert!(!dest.exists());
    }

    #[test]
    fn rejects_url_without_host() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        // A syntactically-valid https url with no host.
        let err = download_model("https:///m.gguf", &"a".repeat(64), &dest, None);
        assert!(matches!(
            err,
            Err(DownloadError::NoHost(_)) | Err(DownloadError::Network(_))
        ));
        assert!(!dest.exists());
    }
}
