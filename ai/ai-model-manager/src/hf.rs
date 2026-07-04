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

/// Search Hugging Face for GGUF models matching `query`, most-downloaded first.
///
/// PRECONDITION: run under `spawn_blocking` (it builds a current-thread runtime
/// for the async SSRF resolver, then a blocking client), like [`crate::fetch::download_model`];
/// it fails loud rather than panicking if called inside a runtime. `limit` is
/// clamped to `[1, 50]`. The `query` is URL-encoded via `query_pairs_mut`, never
/// interpolated raw, so it cannot alter the request.
pub fn search_hf(query: &str, limit: u32) -> Result<Vec<HfHit>, DownloadError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(DownloadError::Runtime(
            "search_hf must run in a synchronous context (use spawn_blocking)".into(),
        ));
    }
    let limit = limit.clamp(1, 50);

    let mut url = reqwest::Url::parse("https://huggingface.co/api/models")
        .map_err(|e| DownloadError::Network(format!("parse url: {e}")))?;
    url.query_pairs_mut()
        .append_pair("search", query)
        .append_pair("filter", "gguf")
        .append_pair("sort", "downloads")
        .append_pair("direction", "-1")
        .append_pair("limit", &limit.to_string());

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
    parse_hf_response(&buf)
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
}
