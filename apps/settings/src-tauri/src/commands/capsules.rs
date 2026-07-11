//! Active-capsules commands (context-capsule.md §8): list the user's context
//! capsules and revoke one by handle, over the `capsuled` owner control socket.
//! The capsule daemon owns mint/serve/revoke and the durable revoke-set; these are
//! the thin transport the privacy page's `capsules.ts` store drives once its fixture
//! is swapped for the live path.
//!
//! `CapsuleControlClient` is synchronous one-shot-per-connection, so each call runs
//! on a blocking thread to keep the async runtime free.

use capsuled::control_client::CapsuleControlClient;
use capsuled::revocation::CapsuleListEntry;
use serde::Serialize;

/// One capsule as the privacy-page list renders it, matching `capsules.ts`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capsule {
    id: String,
    handle: String,
    label: String,
    audience: String,
    scope: String,
    expires_at: String,
    reads_left: i64,
    /// `"active" | "expired" | "exhausted"`.
    state: String,
}

/// Now, in microseconds since the epoch (for the expiry/state derivation).
fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// Render an expiry stamp for a person, or a fallback when it is unknown.
fn format_expiry(micros: i64) -> String {
    if micros <= 0 {
        return "unknown".to_string();
    }
    chrono::DateTime::from_timestamp(micros / 1_000_000, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Map a ledger entry to the list shape, or `None` for a revoked capsule (a revoke
/// removes it from the active view; the frontend has no revoked state).
fn to_capsule(e: CapsuleListEntry, now: i64) -> Option<Capsule> {
    if e.revoked {
        return None;
    }
    let (label, audience, scope, expiry_micros, max_ops) = match &e.meta {
        Some(m) => (
            m.label.clone(),
            m.audience.clone(),
            m.scope.clone(),
            m.expiry_micros,
            m.max_ops,
        ),
        // A metadata-less capsule (bare-registered or pre-metadata) still lists,
        // named by its handle so it can be revoked.
        None => (e.handle.clone(), "this machine".to_string(), String::new(), 0, 0),
    };
    let reads_left = (max_ops as i64 - e.ops_used as i64).max(0);
    let state = if expiry_micros > 0 && now > expiry_micros {
        "expired"
    } else if max_ops > 0 && e.ops_used >= max_ops {
        "exhausted"
    } else {
        "active"
    };
    Some(Capsule {
        id: e.handle.clone(),
        handle: e.handle,
        label,
        audience,
        scope,
        expires_at: format_expiry(expiry_micros),
        reads_left,
        state: state.to_string(),
    })
}

/// List the user's active capsules (revoked ones are omitted). Wraps the daemon's
/// `CapsuleControlClient::list`.
#[tauri::command]
pub async fn list_capsules() -> Result<Vec<Capsule>, String> {
    tokio::task::spawn_blocking(|| {
        let client = CapsuleControlClient::at_default_path().map_err(|e| e.to_string())?;
        let entries = client.list().map_err(|e| e.to_string())?;
        let now = now_micros();
        Ok(entries.into_iter().filter_map(|e| to_capsule(e, now)).collect())
    })
    .await
    .map_err(|e| format!("list_capsules task failed: {e}"))?
}

/// Revoke a capsule by handle (terminal: every future read is refused). Wraps the
/// daemon's `CapsuleControlClient::revoke`.
#[tauri::command]
pub async fn revoke_capsule(handle: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        CapsuleControlClient::at_default_path()
            .map_err(|e| e.to_string())?
            .revoke(&handle)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("revoke_capsule task failed: {e}"))?
}
