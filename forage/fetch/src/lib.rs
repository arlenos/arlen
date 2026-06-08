//! Fixed-output source fetch for forage.
//!
//! Fetch is the one phase with network access (forage-recipes.md sections 9,
//! 10a): the source is downloaded only from the hosts declared in the recipe's
//! `[source]`, SSRF-guarded and DNS-pinned via [`arlen_net_guard`], then stored
//! and rooted into the content-addressed store **only if it matches the
//! recipe's pinned `sha256`/commit**. A mismatch stores nothing. Because the
//! output is verified against a pre-pinned hash, allowing network here is safe
//! (a fixed-output derivation); after this phase the build runs with no network.
//!
//! This slice handles `tarball` sources (HTTP GET + sha256). `git`,
//! `github-release`, `crate` and `local` sources are follow-up slices and
//! return [`FetchError::Unsupported`] for now.

use arlen_forage_recipe::{Source, SourceType};
use arlen_forage_store::{ContentHash, Store, StoreError};
use arlen_net_guard::{resolve_and_pin, GuardError};
use async_trait::async_trait;

/// Default cap on a single downloaded source artifact (1 GiB). Source tarballs
/// are larger than module fetches; a recipe-specific override can come later.
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// A failure fetching or storing a source.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The source kind is not yet fetchable by this slice.
    #[error("unsupported source type: {0:?}")]
    Unsupported(SourceType),
    /// A field required for this source kind is missing.
    #[error("source is missing required field: {0}")]
    MissingField(&'static str),
    /// The declared sha256 is malformed.
    #[error("invalid sha256: {0}")]
    InvalidHash(String),
    /// A network/transport failure.
    #[error("network: {0}")]
    Network(String),
    /// The destination resolved into a blocked range (SSRF guard).
    #[error("blocked destination: {0}")]
    Blocked(String),
    /// The download exceeded the size cap.
    #[error("download exceeded {limit} bytes")]
    TooLarge {
        /// The cap that was exceeded.
        limit: u64,
    },
    /// Storing or verifying the fetched bytes failed (includes a hash mismatch).
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// Fetches the bytes at a URL, capped at `max_bytes`. Behind a trait so the
/// fetch logic can be tested without real network.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// GET `url`, returning at most `max_bytes` of body or [`FetchError::TooLarge`].
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError>;
}

/// Fetch `source` and, if it matches its pinned hash, store and root it to
/// `owner` in the content-addressed store, returning its address. Nothing is
/// stored on a mismatch or any failure.
pub async fn fetch_source(
    source: &Source,
    owner: &str,
    store: &Store,
    downloader: &dyn Downloader,
    max_bytes: u64,
) -> Result<ContentHash, FetchError> {
    match source.source_type {
        SourceType::Tarball => {
            let url = source
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .ok_or(FetchError::MissingField("url"))?;
            let sha = source
                .sha256
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or(FetchError::MissingField("sha256"))?;
            let expected =
                ContentHash::parse(sha).map_err(|_| FetchError::InvalidHash(sha.to_string()))?;
            let bytes = downloader.get(url, max_bytes).await?;
            // The store verifies the bytes against `expected` before writing,
            // so a mismatch stores nothing, and roots atomically on a match.
            let hash = store.put_verified_referenced(&bytes, &expected, owner)?;
            Ok(hash)
        }
        other => Err(FetchError::Unsupported(other)),
    }
}

/// The production [`Downloader`]: reqwest over rustls, HTTPS only, no redirects,
/// the host SSRF-guarded and DNS-pinned (resolve ourselves, pin reqwest to the
/// verified address so rebinding cannot swap the target), with a streamed body
/// capped at `max_bytes`.
#[derive(Debug, Default)]
pub struct HttpDownloader;

#[async_trait]
impl Downloader for HttpDownloader {
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        let parsed =
            reqwest::Url::parse(url).map_err(|e| FetchError::Network(format!("parse url: {e}")))?;
        if parsed.scheme() != "https" {
            return Err(FetchError::Network(format!(
                "non-https source url: {url}"
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::Network(format!("url has no host: {url}")))?;
        let port = parsed.port_or_known_default().unwrap_or(443);

        // SSRF guard + DNS-rebinding pin: resolve ourselves, reject blocked
        // ranges, pin reqwest to the verified address.
        let addr = match resolve_and_pin(host, port).await {
            Ok(addr) => addr,
            Err(blocked @ GuardError::Blocked { .. }) => {
                return Err(FetchError::Blocked(blocked.to_string()))
            }
            Err(e) => return Err(FetchError::Network(e.to_string())),
        };

        let client = reqwest::Client::builder()
            .https_only(true)
            // Ignore any HTTPS_PROXY/ALL_PROXY in the environment: a proxy would
            // be contacted without running through resolve_and_pin, bypassing
            // the SSRF/DNS-rebinding guard. We resolve and pin the origin
            // ourselves, so no implicit proxy may sit in front of it.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(120))
            .user_agent(concat!("Arlen-forage/", env!("CARGO_PKG_VERSION")))
            .resolve(host, addr)
            .build()
            .map_err(|e| FetchError::Network(format!("client: {e}")))?;

        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FetchError::Network(format!("status {}", resp.status())));
        }

        // Stream with a hard cap so a hostile server cannot exhaust memory.
        use futures_util::StreamExt;
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| FetchError::Network(format!("body: {e}")))?;
            if buf.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(FetchError::TooLarge { limit: max_bytes });
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A downloader that returns canned bytes (or a canned error), so the fetch
    /// logic is tested without network.
    struct MockDownloader {
        result: std::sync::Mutex<Option<Result<Vec<u8>, FetchError>>>,
    }
    impl MockDownloader {
        fn ok(bytes: &[u8]) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Ok(bytes.to_vec()))),
            }
        }
        fn err(e: FetchError) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Err(e))),
            }
        }
    }
    #[async_trait]
    impl Downloader for MockDownloader {
        async fn get(&self, _url: &str, _max_bytes: u64) -> Result<Vec<u8>, FetchError> {
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("mock downloader called once")
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        (dir, s)
    }

    fn tarball(url: &str, sha256: Option<&str>) -> Source {
        Source {
            source_type: SourceType::Tarball,
            url: Some(url.to_string()),
            commit: None,
            sha256: sha256.map(|s| s.to_string()),
            asset: None,
            tag: None,
            patches: Vec::new(),
        }
    }

    #[tokio::test]
    async fn matching_tarball_is_stored_and_rooted() {
        let (_d, s) = store();
        let body = b"the source tree";
        let sha = ContentHash::of(body);
        let src = tarball("https://example.org/x.tar.gz", Some(sha.as_str()));
        let dl = MockDownloader::ok(body);
        let h = fetch_source(&src, "org.example.app", &s, &dl, DEFAULT_MAX_BYTES)
            .await
            .unwrap();
        assert_eq!(h, sha);
        assert_eq!(s.refcount(&h).unwrap(), 1);
        assert_eq!(s.read(&h).unwrap(), body);
    }

    #[tokio::test]
    async fn sha256_mismatch_stores_nothing() {
        let (_d, s) = store();
        let declared = ContentHash::of(b"what the recipe pinned");
        let src = tarball("https://example.org/x.tar.gz", Some(declared.as_str()));
        let dl = MockDownloader::ok(b"but the server served this");
        let err = fetch_source(&src, "org.example.app", &s, &dl, DEFAULT_MAX_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Store(StoreError::Mismatch { .. })));
        assert!(!s.has(&declared));
        assert!(!s.has(&ContentHash::of(b"but the server served this")));
    }

    #[tokio::test]
    async fn missing_sha256_is_rejected() {
        let (_d, s) = store();
        let src = tarball("https://example.org/x.tar.gz", None);
        let dl = MockDownloader::ok(b"x");
        assert!(matches!(
            fetch_source(&src, "o", &s, &dl, DEFAULT_MAX_BYTES).await,
            Err(FetchError::MissingField("sha256"))
        ));
    }

    #[tokio::test]
    async fn unsupported_source_type() {
        let (_d, s) = store();
        let mut src = tarball("https://example.org/x", Some(&"a".repeat(64)));
        src.source_type = SourceType::Git;
        let dl = MockDownloader::ok(b"x");
        assert!(matches!(
            fetch_source(&src, "o", &s, &dl, DEFAULT_MAX_BYTES).await,
            Err(FetchError::Unsupported(SourceType::Git))
        ));
    }

    #[tokio::test]
    async fn download_error_propagates_and_stores_nothing() {
        let (_d, s) = store();
        let sha = ContentHash::of(b"never arrives");
        let src = tarball("https://example.org/x.tar.gz", Some(sha.as_str()));
        let dl = MockDownloader::err(FetchError::TooLarge { limit: 10 });
        assert!(matches!(
            fetch_source(&src, "o", &s, &dl, DEFAULT_MAX_BYTES).await,
            Err(FetchError::TooLarge { limit: 10 })
        ));
        assert!(!s.has(&sha));
    }
}
