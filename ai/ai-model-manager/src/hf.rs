//! Live Hugging Face model search (opt-in egress).
//!
//! The Models hub can search Hugging Face for GGUF repositories to install. This
//! is an OPT-IN egress: a caller triggers it by an explicit search action, never
//! silently on keystroke. It does one SSRF-pinned GET to the HF models API (the
//! same `resolve_and_pin` floor `download_model` uses) and parses the result. The
//! API responds directly, so unlike a CDN download there is no redirect chain to
//! follow. Reuses [`crate::fetch::DownloadError`] as the crate's HTTP-fetch error
//! taxonomy (its Blocked / Network / Status / Runtime variants apply to any fetch).

use crate::fetch::DownloadError;
use arlen_net_guard::{resolve_and_pin, GuardError};
use std::io::Read;

/// The HF API host we pin and query.
const HF_HOST: &str = "huggingface.co";
/// Wall-clock bound on the search request.
const HF_SEARCH_TIMEOUT_SECS: u64 = 15;
/// Cap on the JSON body read into memory (the API returns a bounded list, but a
/// hostile/misconfigured origin must not be able to exhaust memory).
const MAX_HF_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

/// One Hugging Face search hit, the subset the hub shows. `id` is the repo id
/// (`org/name`), the value a subsequent download would target.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HfHit {
    /// The repository id, e.g. `bartowski/Llama-3.2-1B-Instruct-GGUF`.
    pub id: String,
    /// All-time downloads, when the API reports them (0 otherwise).
    #[serde(default)]
    pub downloads: u64,
    /// Likes, when the API reports them (0 otherwise).
    #[serde(default)]
    pub likes: u64,
}

/// Parse the HF `/api/models` JSON array into hits. Extra fields are ignored, so
/// the hub stays forward-compatible with the API. Pure + testable (the live GET
/// is not, since the SSRF pin binds the real host).
pub fn parse_hf_response(bytes: &[u8]) -> Result<Vec<HfHit>, DownloadError> {
    serde_json::from_slice(bytes)
        .map_err(|e| DownloadError::Network(format!("parse hf response: {e}")))
}

/// One SSRF-pinned GET to a huggingface.co URL, returning the bounded response
/// body. Shared by the search + file-resolution ops. PRECONDITION: run under
/// `spawn_blocking` (it builds a current-thread runtime for the async SSRF
/// resolver, then a blocking client), like [`crate::fetch::download_model`]; it
/// fails loud rather than panicking if called inside a runtime. The API responds
/// directly, so redirects are disabled (a redirect is treated as a failure).
fn hf_get(url: reqwest::Url) -> Result<Vec<u8>, DownloadError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(DownloadError::Runtime(
            "hf_get must run in a synchronous context (use spawn_blocking)".into(),
        ));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| DownloadError::Runtime(e.to_string()))?;

    // SSRF guard + DNS-rebinding pin for the API host.
    let addr = match rt.block_on(resolve_and_pin(HF_HOST, 443)) {
        Ok(addr) => addr,
        Err(blocked @ GuardError::Blocked { .. }) => {
            return Err(DownloadError::Blocked(blocked.to_string()))
        }
        Err(e) => return Err(DownloadError::Network(e.to_string())),
    };

    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(HF_SEARCH_TIMEOUT_SECS))
        .user_agent(concat!("Arlen-model-manager/", env!("CARGO_PKG_VERSION")))
        .resolve(HF_HOST, addr)
        .build()
        .map_err(|e| DownloadError::Network(format!("client: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DownloadError::Status(resp.status().as_u16()));
    }

    let mut buf = Vec::new();
    resp.take(MAX_HF_RESPONSE_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    Ok(buf)
}

/// Search Hugging Face for GGUF models matching `query`, most-downloaded first.
///
/// PRECONDITION: run under `spawn_blocking` (see [`hf_get`]). `limit` is clamped
/// to `[1, 50]`. The `query` is URL-encoded via `query_pairs_mut`, never
/// interpolated raw, so it cannot alter the request.
pub fn search_hf(query: &str, limit: u32) -> Result<Vec<HfHit>, DownloadError> {
    let limit = limit.clamp(1, 50);
    let mut url = reqwest::Url::parse("https://huggingface.co/api/models")
        .map_err(|e| DownloadError::Network(format!("parse url: {e}")))?;
    url.query_pairs_mut()
        .append_pair("search", query)
        .append_pair("filter", "gguf")
        .append_pair("sort", "downloads")
        .append_pair("direction", "-1")
        .append_pair("limit", &limit.to_string());
    parse_hf_response(&hf_get(url)?)
}

/// A resolved GGUF file's integrity + size, from its repo's HF file tree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResolvedFile {
    /// The sha256 the download must verify against (an LFS blob's oid IS its
    /// sha256), lowercase hex.
    pub sha256: String,
    /// The file size in bytes (for the download progress total + a fit sanity check).
    pub size: u64,
}

/// One entry in the HF `/api/models/{repo}/tree/{rev}` listing. GGUF files are
/// stored via git-LFS, so the authoritative sha256 is `lfs.oid`; a plain
/// (non-LFS) entry has no `lfs` block and is not a resolvable model file.
#[derive(serde::Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(default)]
    lfs: Option<TreeLfs>,
}

#[derive(serde::Deserialize)]
struct TreeLfs {
    oid: String,
    #[serde(default)]
    size: u64,
}

/// Pure: find `filename`'s sha256 + size in a parsed HF tree listing. Returns
/// `None` if the file is absent or is not an LFS blob (so has no oid to verify).
pub fn find_file_in_tree(bytes: &[u8], filename: &str) -> Result<Option<ResolvedFile>, DownloadError> {
    let entries: Vec<TreeEntry> = serde_json::from_slice(bytes)
        .map_err(|e| DownloadError::Network(format!("parse hf tree: {e}")))?;
    Ok(entries.into_iter().find(|e| e.path == filename).and_then(|e| {
        e.lfs.map(|l| ResolvedFile {
            sha256: l.oid,
            size: l.size,
        })
    }))
}

/// Resolve a GGUF file's sha256 + size from its repo's HF file tree (opt-in
/// egress; run under `spawn_blocking`). Does one SSRF-pinned GET of
/// `/api/models/{repo}/tree/main` and looks up `filename`. The sha256 lets the
/// subsequent download verify integrity. Errors if the repo id is unsafe (it is
/// interpolated into the path), the file is absent, or the request fails.
pub fn resolve_gguf_sha(repo: &str, filename: &str) -> Result<ResolvedFile, DownloadError> {
    if !crate::download::is_valid_hf_repo(repo) {
        return Err(DownloadError::Network(format!("unsafe repo id: {repo}")));
    }
    let url = reqwest::Url::parse(&format!("https://huggingface.co/api/models/{repo}/tree/main"))
        .map_err(|e| DownloadError::Network(format!("parse url: {e}")))?;
    find_file_in_tree(&hf_get(url)?, filename)?
        .ok_or_else(|| DownloadError::Network(format!("file not found in repo tree: {filename}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_hf_models_response() {
        let body = br#"[
            {"id":"bartowski/Llama-3.2-1B-Instruct-GGUF","downloads":12345,"likes":42,"private":false},
            {"id":"someone/Qwen2.5-7B-GGUF","downloads":99}
        ]"#;
        let hits = parse_hf_response(body).expect("parse");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "bartowski/Llama-3.2-1B-Instruct-GGUF");
        assert_eq!(hits[0].downloads, 12345);
        assert_eq!(hits[0].likes, 42);
        // Missing likes defaults to 0.
        assert_eq!(hits[1].downloads, 99);
        assert_eq!(hits[1].likes, 0);
    }

    #[test]
    fn rejects_a_non_array_response() {
        assert!(parse_hf_response(b"not json").is_err());
        assert!(parse_hf_response(br#"{"error":"nope"}"#).is_err());
    }

    #[test]
    fn refuses_to_run_inside_a_tokio_runtime() {
        // Called from within a runtime it must fail loud, not panic (a future async
        // caller then knows to wrap it in spawn_blocking).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(async { search_hf("llama", 10) });
        assert!(matches!(err, Err(DownloadError::Runtime(_))));
    }

    #[test]
    fn finds_an_lfs_file_sha_in_a_tree() {
        let body = br#"[
            {"path":"README.md","size":1234},
            {"path":"Llama-3.2-1B-Instruct-Q4_K_M.gguf","size":808,"lfs":{"oid":"abc123def","size":770000000}}
        ]"#;
        let found = find_file_in_tree(body, "Llama-3.2-1B-Instruct-Q4_K_M.gguf")
            .expect("parse")
            .expect("found");
        assert_eq!(found.sha256, "abc123def");
        assert_eq!(found.size, 770_000_000);
        // Absent file -> None.
        assert!(find_file_in_tree(body, "missing.gguf").unwrap().is_none());
        // A non-LFS entry has no verifiable oid, so it is not resolvable.
        assert!(find_file_in_tree(body, "README.md").unwrap().is_none());
    }
}
