//! Provider/model selection vocabulary for the live-switch picker
//! (coder-jobs "AI provider/model selection - the live-switch backend").
//!
//! The harness reads the catalog (`ai_models_list`), the current live selection
//! (`ai_active`), and live-swaps it (`ai_set_active`). All three key on the
//! **(provider, model) pair, never the model alone**: one model id may appear
//! under several providers, so the pair is the identity.
//!
//! These are the wire shapes the three D-Bus commands share, defined here once
//! (contract-first, like the audit-proto activity types). Serialized camelCase to
//! match every other daemon-to-harness DTO in this tree (`ActivityEntry`,
//! `PendingProposal`, `ToolCall`); the harness deserializes the same casing.

use serde::{Deserialize, Serialize};

/// Whether a model runs on the local machine or a remote cloud service. Drives
/// the picker's local/cloud grouping; cloud entries arrive as those providers
/// land (Phase 9-β/γ), so today the catalog is local-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    /// Runs on this machine (e.g. an Ollama model).
    Local,
    /// Runs on a remote service reached over the network.
    Cloud,
}

/// One catalogued provider+model the picker can offer, with the display metadata
/// the harness shows. `available` is the provider's reachability at list time (a
/// down local backend lists its models as unavailable rather than dropping them,
/// so the picker can show why a choice is greyed out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    /// The provider name (the proxy catalog key).
    pub provider: String,
    /// The model id within that provider.
    pub model: String,
    /// The model's context window in tokens, for the compaction bound.
    pub context_window: u32,
    /// Local vs cloud, for the picker's grouping.
    pub kind: ModelKind,
    /// Whether the provider was reachable when the catalog was built.
    pub available: bool,
}

/// The live provider+model the daemon is currently routing to. Keyed on the
/// pair. This is the daemon's runtime state, NOT `ai.toml` (Settings owns the
/// file; the in-chat swap overrides it for the live session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSelection {
    /// The active provider name.
    pub provider: String,
    /// The active model id.
    pub model: String,
}

impl ActiveSelection {
    /// A selection of `provider`/`model`.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        ActiveSelection {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// Validate a requested (provider, model) against the catalog for `ai_set_active`:
/// the exact pair must be catalogued AND available. Returns the matched entry so
/// the caller can apply its `context_window`. Fail-closed: a pair absent from the
/// catalog, or present but unavailable, is refused (no swap to a backend that is
/// not there). Pure, so the swap's guard is tested without the daemon.
pub fn validate_selection<'a>(
    catalog: &'a [ModelEntry],
    provider: &str,
    model: &str,
) -> Option<&'a ModelEntry> {
    catalog
        .iter()
        .find(|e| e.provider == provider && e.model == model && e.available)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str, available: bool) -> ModelEntry {
        ModelEntry {
            provider: provider.into(),
            model: model.into(),
            context_window: 8192,
            kind: ModelKind::Local,
            available,
        }
    }

    #[test]
    fn validate_matches_only_the_available_pair() {
        let catalog = vec![
            entry("ollama-default", "llama3:8b", true),
            entry("ollama-default", "mistral", false),
            entry("other", "llama3:8b", true),
        ];
        // Exact available pair matches.
        assert_eq!(
            validate_selection(&catalog, "ollama-default", "llama3:8b").map(|e| e.model.as_str()),
            Some("llama3:8b")
        );
        // Same model under a different provider is a distinct, selectable pair.
        assert!(validate_selection(&catalog, "other", "llama3:8b").is_some());
        // Catalogued but unavailable is refused.
        assert!(validate_selection(&catalog, "ollama-default", "mistral").is_none());
        // Absent pair is refused.
        assert!(validate_selection(&catalog, "ollama-default", "ghost").is_none());
        // Right model, wrong provider (pair, not model alone).
        assert!(validate_selection(&catalog, "nope", "llama3:8b").is_none());
    }

    #[test]
    fn wire_shapes_round_trip_camelcase() {
        let e = entry("ollama-default", "llama3:8b", true);
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"contextWindow\":8192"));
        assert!(json.contains("\"kind\":\"local\""));
        let back: ModelEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);

        let a = ActiveSelection::new("ollama-default", "llama3:8b");
        let back: ActiveSelection = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
