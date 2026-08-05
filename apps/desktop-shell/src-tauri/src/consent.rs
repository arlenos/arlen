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
    tokio::task::spawn_blocking(|| ControlClient::at_default_path()?.fetch())
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
    // Logged on the RUST side on purpose. The webview's `console.error` does not
    // reach the journal on the image, so a failure in the click path was
    // indistinguishable from a click that never happened - the dialog simply
    // stood there. This line proves the invoke ARRIVED; its absence proves it did
    // not, which is the one thing the boot evidence could not tell apart.
    log::info!("consent_resolve: id={id} outcome={outcome:?}");
    let result = tokio::task::spawn_blocking(move || ControlClient::at_default_path()?.resolve(id, outcome))
        .await
        .map_err(|e| format!("consent resolve task failed: {e}"))?
        .map_err(|e| format!("consent resolve: {e}"));
    match &result {
        Ok(ok) => log::info!("consent_resolve: broker answered ok={ok}"),
        Err(e) => log::error!("consent_resolve: {e}"),
    }
    result
}

/// Route a log line from the shell's frontend into the Rust logger, so it lands in
/// the journal beside the backend lines.
///
/// The shell is the one surface with no way to open devtools on the image, and its
/// `console.error` goes nowhere a boot log can see. That gap is what made the
/// standing consent dialog undiagnosable for several boots: the click path could
/// have failed at four different places and every one of them looked identical
/// from outside. `apps/files` and `apps/harness` already carry this command; the
/// shell, which needs it most, did not.
#[tauri::command]
pub fn frontend_log(level: String, msg: String) {
    match level.as_str() {
        "warn" => log::warn!("[frontend] {msg}"),
        "error" => log::error!("[frontend] {msg}"),
        _ => log::info!("[frontend] {msg}"),
    }
}
