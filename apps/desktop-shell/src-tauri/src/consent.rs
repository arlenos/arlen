//! Tauri commands wrapping the consent broker's control client, so the shell's
//! consent dialog can fetch the front pending request and submit the user's
//! decision. The broker (daemons/consent-broker) attests the shell peer over
//! SO_PEERCRED and owns the queue, severity-tier resolution, grant persistence
//! and audit; these commands are the thin transport the dialog store
//! (`lib/stores/consent.ts`) drives once its fixture is swapped for the live path.
//!
//! `ControlClient` is a synchronous one-shot-per-connection client, so each call
//! runs on a blocking thread to keep the async runtime free.

use arlen_consent_broker::control::PendingView;
use arlen_consent_broker::control_client::ControlClient;
use arlen_consent_broker::ConsentOutcome;

/// Fetch the front pending consent request to render, or `None` when nothing is
/// pending. Wraps the broker's `ControlClient::fetch`.
#[tauri::command]
pub async fn consent_fetch() -> Result<Option<PendingView>, String> {
    tokio::task::spawn_blocking(|| ControlClient::at_default_path().fetch())
        .await
        .map_err(|e| format!("consent fetch task failed: {e}"))?
        .map_err(|e| format!("consent fetch: {e}"))
}

/// Submit the user's decision for a pending request. Returns `false` if the id was
/// unknown or already resolved. Wraps the broker's `ControlClient::resolve`, which
/// removes it from the queue, replies to the waiting requester and persists a
/// grant for an always-allow.
#[tauri::command]
pub async fn consent_resolve(id: u64, outcome: ConsentOutcome) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || ControlClient::at_default_path().resolve(id, outcome))
        .await
        .map_err(|e| format!("consent resolve task failed: {e}"))?
        .map_err(|e| format!("consent resolve: {e}"))
}
