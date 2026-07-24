//! The cached popular-GGUF index: the on-disk data contract for MP-6's
//! recommendation pool.
//!
//! The picker's "runs well on your machine" section draws its candidates from the
//! whole HuggingFace ecosystem, not a hardcoded list, but must NOT hammer HF on
//! every visit. A background refresh job (its home - a crate-side job or a
//! daemon-side scheduled task - is a separate call) searches the top popular GGUF
//! repos, resolves each candidate's real GGUF file size once, and writes THIS
//! index. The picker then opens instantly, fit-ranking the cached sizes locally
//! (see [`crate::fit_badge_from_size`]); offline it falls back to the last cached
//! index plus the bundled seed. This module is only the shared contract - the
//! serde schema, the staleness policy, and the entry-level fit - so both the writer
//! (the refresh job) and the reader (the picker) agree on the format without a
//! network or a home decision.

use crate::{fit_badge_from_size, FitBadge, Hardware};

/// One cached candidate: a HuggingFace GGUF repo, the params parsed from its id, a
/// representative GGUF file's resolved on-disk size (what the size-aware fit ranks
/// against), and its popularity (the quality/trust proxy for HF-derived picks, as
/// the curated set is hand-vetted). `file_size_bytes` is 0 when the refresh job
/// could not resolve a size, in which case the reader falls back to the params-only
/// name-heuristic estimate rather than trusting a zero size.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IndexedModel {
    /// The repository id, e.g. `bartowski/Qwen2.5-7B-Instruct-GGUF`.
    pub id: String,
    /// The parameter count in billions, parsed from the id at refresh time.
    pub params_b: f64,
    /// The representative GGUF file's size in bytes (0 = unresolved).
    #[serde(default)]
    pub file_size_bytes: u64,
    /// All-time downloads (the popularity/trust proxy).
    #[serde(default)]
    pub downloads: u64,
    /// Likes, when the API reports them.
    #[serde(default)]
    pub likes: u64,
}

impl IndexedModel {
    /// Whether a real GGUF size was resolved for this entry. A `false` here means
    /// the reader must fall back to the params-only estimate, never rank a zero.
    pub fn has_resolved_size(&self) -> bool {
        self.file_size_bytes > 0
    }

    /// The size-aware fit verdict for this cached entry on `hw`, or `None` when no
    /// GGUF size was resolved (the caller then uses the name-heuristic estimate).
    pub fn fit(&self, hw: &Hardware) -> Option<FitBadge> {
        self.has_resolved_size()
            .then(|| fit_badge_from_size(self.file_size_bytes, hw))
    }
}

/// The cached popular-GGUF index as written by the refresh job and read by the
/// picker. `captured_at_unix` stamps when the pool was fetched, so the reader can
/// decide whether to trigger a refresh or open on the cache as-is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CachedIndex {
    /// Unix seconds when this index was captured (the freshness anchor).
    pub captured_at_unix: u64,
    /// The candidate pool, most-popular first (the writer's ordering).
    pub models: Vec<IndexedModel>,
}

impl CachedIndex {
    /// Parse a cached index from its JSON bytes. Pure, so the on-disk read is a thin
    /// wrapper the caller owns.
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Serialize this index to pretty JSON for the cache file.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Whether the index is older than `ttl_secs` at `now_unix`, i.e. a refresh is
    /// due. A `captured_at_unix` in the future (a clock adjustment) reads as fresh,
    /// never as stale, so a skewed clock cannot trigger a refresh storm.
    pub fn is_stale(&self, now_unix: u64, ttl_secs: u64) -> bool {
        now_unix.saturating_sub(self.captured_at_unix) >= ttl_secs
    }

    /// Whether the pool is empty (a never-populated or cleared index), so the reader
    /// falls back to the bundled seed alone.
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Accelerator;

    fn apu() -> Hardware {
        Hardware {
            ram_gib: 61.0,
            accelerator: Accelerator::Apu,
            mem_bandwidth_gbps: 102.4,
        }
    }

    fn model(id: &str, params_b: f64, size_bytes: u64) -> IndexedModel {
        IndexedModel {
            id: id.into(),
            params_b,
            file_size_bytes: size_bytes,
            downloads: 1000,
            likes: 10,
        }
    }

    #[test]
    fn round_trips_through_json() {
        let index = CachedIndex {
            captured_at_unix: 1_700_000_000,
            models: vec![
                model("bartowski/Qwen2.5-7B-Instruct-GGUF", 7.0, 4_700_000_000),
                model("bartowski/Llama-3.2-1B-Instruct-GGUF", 1.0, 800_000_000),
            ],
        };
        let json = index.to_json().unwrap();
        assert_eq!(CachedIndex::parse(json.as_bytes()).unwrap(), index);
    }

    #[test]
    fn missing_optional_fields_default_and_an_unresolved_size_reads_as_no_fit() {
        // A minimal entry (no size, no popularity) parses, and its fit is None so
        // the reader falls back to the name-heuristic estimate rather than ranking 0.
        let json = r#"{"captured_at_unix":10,"models":[{"id":"a/b-7B-GGUF","params_b":7.0}]}"#;
        let index = CachedIndex::parse(json.as_bytes()).unwrap();
        let m = &index.models[0];
        assert_eq!(m.file_size_bytes, 0);
        assert_eq!(m.downloads, 0);
        assert!(!m.has_resolved_size());
        assert_eq!(m.fit(&apu()), None);
    }

    #[test]
    fn staleness_is_ttl_bounded_and_future_capture_reads_fresh() {
        let index = CachedIndex {
            captured_at_unix: 1000,
            models: vec![],
        };
        assert!(!index.is_stale(1000 + 3599, 3600));
        assert!(index.is_stale(1000 + 3600, 3600));
        // A capture stamped in the future (clock moved back) is never stale.
        assert!(!index.is_stale(500, 3600));
    }

    #[test]
    fn a_resolved_entry_fits_on_the_apu() {
        let m = model("bartowski/Qwen2.5-7B-Instruct-GGUF", 7.0, 4_700_000_000);
        assert_eq!(m.fit(&apu()), Some(FitBadge::Fits));
    }
}
