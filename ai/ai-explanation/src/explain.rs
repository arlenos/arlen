//! Orchestration: snapshot to plain-language summary via the provider.
//!
//! [`explain`] is the source-agnostic core: it takes an already
//! assembled [`SystemSnapshot`], builds the tagged prompt, and runs one
//! provider completion, returning the model's summary text. The daemon
//! (a later increment) assembles a full snapshot from every source (the
//! graph context, the live event stream, the Anomaly Detector) and
//! calls [`explain`]; [`explain_system`] is the convenience that wires
//! the graph-context source for callers that only need that half today.
//!
//! Foundation §5.8: the summary is generated locally unless the user
//! configured a cloud provider, and only on demand. This module makes
//! no policy decision about which provider runs; it calls the one it is
//! given.

use lunaris_ai_core::provider::{AIProvider, CompletionRequest};

use crate::prompt::build_explanation_prompt;
use crate::snapshot::SystemSnapshot;
use crate::source::{graph_context, GraphReader, SnapshotError};

/// Advisory cap on the summary length. The explanation is a few
/// sentences; bounding the output keeps it concise and the cost (on a
/// cloud provider) predictable. Adapters that honour `extras.max_tokens`
/// forward it upstream.
const SUMMARY_MAX_TOKENS: u32 = 400;

/// An error producing the explanation.
#[derive(Debug, thiserror::Error)]
pub enum ExplainError {
    /// Assembling the snapshot from the graph failed.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// The provider completion failed.
    #[error("provider failed: {0}")]
    Provider(String),
}

/// Summarise an assembled snapshot in plain language via `provider`.
/// Builds the S18-A-tagged prompt and runs a single completion. The
/// snapshot is whatever the caller assembled; an empty (quiet) snapshot
/// still produces a valid "system is idle" summary.
pub async fn explain(
    snapshot: &SystemSnapshot,
    provider: &dyn AIProvider,
) -> Result<String, ExplainError> {
    let prompt = build_explanation_prompt(snapshot);
    let request = CompletionRequest {
        prompt,
        extras: serde_json::json!({ "max_tokens": SUMMARY_MAX_TOKENS }),
    };
    let response = provider
        .complete(request)
        .await
        .map_err(|e| ExplainError::Provider(e.to_string()))?;
    Ok(response.text)
}

/// Convenience: assemble the graph-context half of a snapshot through
/// `reader`, then [`explain`] it. For callers that only have the graph
/// source wired today (the live-moment and anomaly sources fold in at
/// the daemon once they exist). `now_unix` stamps the snapshot.
pub async fn explain_system(
    reader: &dyn GraphReader,
    provider: &dyn AIProvider,
    now_unix: i64,
) -> Result<String, ExplainError> {
    let snapshot = graph_context(reader, now_unix).await?;
    explain(&snapshot, provider).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use lunaris_ai_core::provider::{CompletionResponse, ProviderAudit, ProviderError};
    use std::sync::Mutex;

    /// A provider that records the prompt it received and returns a
    /// canned summary (or a canned error).
    struct MockProvider {
        reply: Result<String, ProviderError>,
        seen_prompt: Mutex<Option<String>>,
    }

    impl MockProvider {
        fn ok(text: &str) -> Self {
            Self {
                reply: Ok(text.to_string()),
                seen_prompt: Mutex::new(None),
            }
        }
        fn err(e: ProviderError) -> Self {
            Self {
                reply: Err(e),
                seen_prompt: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl AIProvider for MockProvider {
        async fn complete(
            &self,
            req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            *self.seen_prompt.lock().unwrap() = Some(req.prompt.clone());
            self.reply.clone().map(|text| CompletionResponse {
                text,
                audit: ProviderAudit {
                    provider_name: "mock".into(),
                    model: "mock".into(),
                    input_tokens: None,
                    output_tokens: None,
                },
            })
        }
        async fn available(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "mock"
        }
    }

    fn quiet() -> SystemSnapshot {
        SystemSnapshot {
            captured_at_unix: 1,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn explain_returns_the_provider_summary_and_sends_a_tagged_prompt() {
        let provider = MockProvider::ok("Your computer is idle.");
        let summary = explain(&quiet(), &provider).await.unwrap();
        assert_eq!(summary, "Your computer is idle.");
        // The prompt the provider saw is the tagged explanation prompt.
        let prompt = provider.seen_prompt.lock().unwrap().clone().unwrap();
        assert!(prompt.contains("What is my computer doing right now?"));
        assert!(prompt.contains("[GRAPH-DATA-"));
    }

    #[tokio::test]
    async fn explain_maps_a_provider_error() {
        let provider = MockProvider::err(ProviderError::Unavailable("no daemon".into()));
        let err = explain(&quiet(), &provider).await.unwrap_err();
        assert!(matches!(err, ExplainError::Provider(_)));
    }
}
