//! A live-swappable [`AIProvider`] wrapper for the provider/model picker
//! (coder-jobs "AI provider/model selection - the live-switch backend").
//!
//! The daemon's provider was built once at startup and moved by value into three
//! consumers (the Cypher pipeline, the explain path, the tool loop), each holding
//! its own `Arc<dyn AIProvider>` - so it was fixed for the daemon's lifetime.
//! `LiveProvider` is the indirection that makes `ai_set_active` a real LIVE swap:
//! the three consumers each hold a clone of the SAME `Arc<LiveProvider>`, and one
//! [`swap`](LiveProvider::swap) changes the backend all three route to, no restart.
//!
//! The backend lives behind a short-held `RwLock`. A completion clones the inner
//! `Arc` under the lock and drops the guard before awaiting, so the lock is never
//! held across `.await`: a concurrent swap is not blocked by an in-flight call,
//! and an in-flight call finishes on the backend it started with (it cloned that
//! `Arc`). The live (provider, model) is surfaced for `ai_active` via
//! [`active`](LiveProvider::active).

use std::sync::{Arc, Mutex, RwLock};

use arlen_ai_core::provider::{AIProvider, CompletionRequest, CompletionResponse, ProviderError};
use async_trait::async_trait;

use crate::selection::ActiveSelection;

/// A swappable wrapper around the daemon's current [`AIProvider`].
pub struct LiveProvider {
    /// The current backend. Read-cloned per call; replaced by `swap`.
    inner: RwLock<Arc<dyn AIProvider>>,
    /// The live (provider, model), kept in step with `inner`.
    active: Mutex<ActiveSelection>,
    /// A stable wrapper label for [`AIProvider::name`]. The live identity is
    /// [`active`](Self::active); the real per-call provider name (for the proxy
    /// forward and its egress audit) is the inner backend's own, unaffected by
    /// this label, so the label staying at the startup provider is harmless.
    label: String,
}

impl LiveProvider {
    /// Wrap `inner` as the initial backend with `active` as its selection.
    pub fn new(inner: Arc<dyn AIProvider>, active: ActiveSelection) -> Self {
        let label = active.provider.clone();
        LiveProvider {
            inner: RwLock::new(inner),
            active: Mutex::new(active),
            label,
        }
    }

    /// The current live (provider, model), cloned for the caller. Backs `ai_active`.
    pub fn active(&self) -> ActiveSelection {
        self.active
            .lock()
            .expect("live-provider active lock poisoned")
            .clone()
    }

    /// Swap the backing provider and record the new selection atomically from a
    /// reader's view (the active lock is held across both writes, so `active`
    /// never reports a half-applied swap). Subsequent completions route to
    /// `inner`; in-flight ones finish on the previous backend.
    pub fn swap(&self, inner: Arc<dyn AIProvider>, active: ActiveSelection) {
        let mut selection = self.active.lock().expect("live-provider active lock poisoned");
        *self.inner.write().expect("live-provider inner lock poisoned") = inner;
        *selection = active;
    }

    /// Clone the current backend `Arc` (lock held only for the clone).
    fn current(&self) -> Arc<dyn AIProvider> {
        self.inner
            .read()
            .expect("live-provider inner lock poisoned")
            .clone()
    }
}

#[async_trait]
impl AIProvider for LiveProvider {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        // Clone the backend Arc, drop the lock, then await: the lock is never
        // held across the await.
        self.current().complete(req).await
    }

    async fn available(&self) -> bool {
        self.current().available().await
    }

    fn name(&self) -> &str {
        &self.label
    }

    fn context_window(&self) -> u32 {
        self.current().context_window()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_core::provider::ProviderAudit;

    /// A provider that echoes its own name + a fixed context window, so a test
    /// can tell which backend a `LiveProvider` routed to.
    struct Marker {
        name: String,
        window: u32,
    }

    #[async_trait]
    impl AIProvider for Marker {
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                text: format!("from:{}", self.name),
                audit: ProviderAudit {
                    provider_name: self.name.clone(),
                    model: self.name.clone(),
                    input_tokens: None,
                    output_tokens: None,
                },
            })
        }
        async fn available(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            &self.name
        }
        fn context_window(&self) -> u32 {
            self.window
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            prompt: "hi".into(),
            extras: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn swap_changes_the_routed_backend_and_active_selection() {
        let live = LiveProvider::new(
            Arc::new(Marker {
                name: "ollama-default".into(),
                window: 8192,
            }),
            ActiveSelection::new("ollama-default", "llama3:8b"),
        );

        // Initial backend.
        assert_eq!(live.active(), ActiveSelection::new("ollama-default", "llama3:8b"));
        assert_eq!(live.context_window(), 8192);
        assert_eq!(live.complete(req()).await.unwrap().text, "from:ollama-default");

        // Swap to a different backend + selection.
        live.swap(
            Arc::new(Marker {
                name: "other".into(),
                window: 4096,
            }),
            ActiveSelection::new("other", "mistral"),
        );

        // The swap is visible to all three reads (active, window, completion).
        assert_eq!(live.active(), ActiveSelection::new("other", "mistral"));
        assert_eq!(live.context_window(), 4096);
        assert_eq!(live.complete(req()).await.unwrap().text, "from:other");
    }
}
