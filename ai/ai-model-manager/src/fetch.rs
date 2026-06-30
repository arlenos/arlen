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

use arlen_net_guard::{resolve_and_pin, GuardError};

use crate::store::{verify_and_store, StoreError};

/// Maximum redirect hops followed before giving up. HF's resolve URL needs one
/// (origin -> CDN); the rest is slack. Each hop is independently SSRF-pinned.
const MAX_REDIRECT_HOPS: usize = 5;
/// Wall-clock bound on the whole transfer. A GGUF is hundreds of MB to a few GB,
/// so this is generous for a slow link while still killing a hung server.
const FETCH_TIMEOUT_SECS: u64 = 3600;
/// Hard cap on the streamed body, so a runaway or compromised CDN cannot fill
/// the disk inside the timeout window. Far above the largest catalog model
/// (a Q-quant 32B GGUF is ~20 GB); a real file over this would truncate and
/// fail the sha check (and be discarded), which is the safe direction.
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024 * 1024;

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
        }
    }
}

impl std::error::Error for DownloadError {}

/// Download a GGUF from `url` to `dest`, verifying it hashes to
/// `expected_sha256` (the caller's pin: a catalog entry or HF file metadata).
/// SSRF-pinned and redirect-following with a per-hop re-pin; the body streams
/// straight into the verified store, never buffered. On any failure `dest` is
/// left untouched (the verified store discards a partial or mismatched file).
pub fn download_model(
    url: &str,
    expected_sha256: &str,
    dest: &Path,
) -> Result<(), DownloadError> {
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
        return verify_and_store(resp.take(MAX_MODEL_BYTES), expected_sha256, dest)
            .map_err(DownloadError::Store);
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
        let err = download_model("http://example.com/m.gguf", &"a".repeat(64), &dest).unwrap_err();
        assert!(matches!(err, DownloadError::NotHttps(_)));
        assert!(!dest.exists());
    }

    #[test]
    fn rejects_url_without_host() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        // A syntactically-valid https url with no host.
        let err = download_model("https:///m.gguf", &"a".repeat(64), &dest);
        assert!(matches!(
            err,
            Err(DownloadError::NoHost(_)) | Err(DownloadError::Network(_))
        ));
        assert!(!dest.exists());
    }
}
