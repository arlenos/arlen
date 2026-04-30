//! Tauri commands. Thin wrappers over `os_sdk::UnixClipboardClient`
//! plus a per-webview subscription manager that emits
//! `lunaris://clipboard-changed` events scoped to the window that
//! called `subscribe`.

use std::collections::HashMap;
use std::sync::Arc;

use os_sdk::UnixClipboardClient;
use tauri::{Emitter, Runtime, State, WebviewWindow};
use tokio::sync::Mutex;

use crate::types::{ClipboardEntry, ClipboardError, WriteParams};

/// Plugin-level state. The shared `client` services request/
/// response calls (`write`/`read`/`history`); subscriptions are
/// keyed by window label so each subscribing window gets its own
/// dedicated SDK stream and only that window receives events.
pub struct ClipboardState {
    client: Mutex<Option<Arc<UnixClipboardClient>>>,
    subscriptions: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl ClipboardState {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
            subscriptions: Mutex::new(HashMap::new()),
        }
    }

    async fn client(&self) -> Result<Arc<UnixClipboardClient>, ClipboardError> {
        let mut guard = self.client.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }
        let new_client = UnixClipboardClient::connect()
            .await
            .map_err(ClipboardError::from)?;
        let arc = Arc::new(new_client);
        *guard = Some(arc.clone());
        Ok(arc)
    }
}

#[tauri::command]
pub async fn write(
    state: State<'_, ClipboardState>,
    params: WriteParams,
) -> Result<(), ClipboardError> {
    let client = state.client().await?;
    let sdk_params = os_sdk::WriteParams {
        content: params.content,
        mime: params.mime,
        label: params.label.into(),
    };
    client.write(sdk_params).await?;
    Ok(())
}

#[tauri::command]
pub async fn read(
    state: State<'_, ClipboardState>,
) -> Result<Option<ClipboardEntry>, ClipboardError> {
    let client = state.client().await?;
    let entry = client.read().await?;
    Ok(entry.map(ClipboardEntry::from))
}

#[tauri::command]
pub async fn history(
    state: State<'_, ClipboardState>,
    limit: Option<u32>,
) -> Result<Vec<ClipboardEntry>, ClipboardError> {
    let client = state.client().await?;
    let entries = client.history(limit.unwrap_or(50)).await?;
    Ok(entries.into_iter().map(ClipboardEntry::from).collect())
}

/// Start a subscription scoped to the calling window. Subsequent
/// clipboard changes are emitted as `lunaris://clipboard-changed`
/// events to that window only — sibling windows in the same app
/// do not receive the payload unless they call `subscribe()`
/// themselves. Calling twice from the same window without an
/// intervening `unsubscribe` returns `AlreadySubscribed`.
#[tauri::command]
pub async fn subscribe<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, ClipboardState>,
) -> Result<(), ClipboardError> {
    let label = window.label().to_string();
    let mut guard = state.subscriptions.lock().await;
    if guard.contains_key(&label) {
        return Err(ClipboardError::AlreadySubscribed);
    }

    // Each subscribe call opens a dedicated SDK stream so the
    // shared client stays free for request/response traffic.
    let client = state.client().await?;
    let mut rx = client.subscribe().await.map_err(ClipboardError::from)?;

    let target = window.clone();
    let handle = tokio::spawn(async move {
        while let Some(entry) = rx.recv().await {
            let payload: ClipboardEntry = entry.into();
            if let Err(err) = target.emit("lunaris://clipboard-changed", payload) {
                log::warn!("clipboard subscribe emit failed: {err}");
            }
        }
    });

    guard.insert(label, handle);
    Ok(())
}

#[tauri::command]
pub async fn unsubscribe<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, ClipboardState>,
) -> Result<(), ClipboardError> {
    let label = window.label().to_string();
    let mut guard = state.subscriptions.lock().await;
    match guard.remove(&label) {
        Some(handle) => {
            handle.abort();
            Ok(())
        }
        None => Err(ClipboardError::NotSubscribed),
    }
}
