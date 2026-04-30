/// Tauri command handlers for the Lunaris shell plugin.
///
/// Each command is a thin wrapper that takes typed parameters from the
/// Tauri frontend (deserialised by Tauri's command machinery) and
/// forwards them to the matching `os-sdk` shell surface. Errors from
/// the SDK are stringified for the Tauri error channel because Tauri
/// commands cannot return arbitrary Rust error types.

use os_sdk::{
    AnnotationLookup, AnnotationRecord, AnnotationSetParams, AnnotationTarget,
    PresenceParams, SpatialHint, TimelineParams,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

use crate::{ShellState, SubscriptionSlot};

#[tauri::command]
pub async fn presence_set<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    params: PresenceParams,
) -> Result<(), String> {
    state
        .presence
        .set(params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn presence_clear<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
) -> Result<(), String> {
    state.presence.clear().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn timeline_record<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    params: TimelineParams,
) -> Result<(), String> {
    state
        .timeline
        .record(params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spatial_hint<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    hint: SpatialHint,
) -> Result<(), String> {
    state.spatial.hint(hint).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn annotation_set<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    params: AnnotationSetParams,
) -> Result<(), String> {
    state
        .annotations
        .set(params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn annotation_clear<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    lookup: AnnotationLookup,
) -> Result<(), String> {
    state
        .annotations
        .clear(lookup)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn annotation_get<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: State<'_, ShellState>,
    lookup: AnnotationLookup,
) -> Result<Option<AnnotationRecord>, String> {
    state
        .annotations
        .get(lookup)
        .await
        .map_err(|e| e.to_string())
}

/// Parameters for [`annotation_subscribe_prepare`]. Same shape
/// as `AnnotationLookup` but accepted as its own type so the
/// frontend can pass `target` and `namespace` cleanly without
/// re-using the read-side lookup struct.
#[derive(Debug, Deserialize)]
pub struct AnnotationSubscribeParams {
    pub target: AnnotationTarget,
    pub namespace: String,
}

/// Two-step subscribe — phase 1.
///
/// Opens the bus subscription, parks it in the `annotation_subs`
/// map in `Pending` state, and returns the subscription id. The
/// SDK forwarder is already pumping events into an internal
/// buffer (the mpsc receiver inside the slot), so events that
/// happen between this call and `annotation_subscribe_start`
/// are NOT lost — they accumulate in the buffer and flow out as
/// soon as the pump task starts.
///
/// The frontend MUST register its `listen()` handler before
/// calling `annotation_subscribe_start`. That ordering is what
/// closes the listener-registration race surfaced by the
/// adversarial review.
#[tauri::command]
pub async fn annotation_subscribe_prepare<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    state: State<'_, ShellState>,
    params: AnnotationSubscribeParams,
) -> Result<String, String> {
    let subscription_id = uuid::Uuid::now_v7().to_string();
    let window_label = window.label().to_string();
    let key = (window_label.clone(), subscription_id.clone());

    let subscription = state
        .annotations
        .on_changed(&state.consumer, params.target, params.namespace)
        .await
        .map_err(|e| e.to_string())?;
    let (abort_on_drop, rx) = subscription.split();

    // Insert the slot in Pending state — abort guard owns the
    // forwarder task; rx owned here keeps events buffered until
    // start() spawns the pump.
    {
        let mut subs = state.annotation_subs.lock().await;
        subs.insert(
            key.clone(),
            SubscriptionSlot {
                abort_on_drop,
                rx: Some(rx),
            },
        );
    }

    // Race-window-destroy fence. If `WindowEvent::Destroyed`
    // fired during the `on_changed().await` above, the cleanup
    // hook may have run before we inserted, finding nothing —
    // and we'd now be holding a slot for a dead window. Re-check
    // window liveness via the AppHandle, and if the window is
    // gone, drop the slot ourselves so the forwarder is aborted
    // and no leak survives. This pairs with the cleanup hook in
    // `lib.rs` which takes the same Mutex; either path tears the
    // entry down.
    if app.get_webview_window(&window_label).is_none() {
        let mut subs = state.annotation_subs.lock().await;
        subs.remove(&key);
        return Err("window closed during subscribe".into());
    }

    Ok(subscription_id)
}

/// Two-step subscribe — phase 2.
///
/// Spawns the pump task that drains the prepared subscription's
/// receiver and emits per-webview Tauri events. By this point
/// the frontend has registered its `listen()` handler, so the
/// pump's first emits land on a live listener. Events that were
/// buffered during the prepare-to-start window flush first.
#[tauri::command]
pub async fn annotation_subscribe_start<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    state: State<'_, ShellState>,
    subscription_id: String,
) -> Result<(), String> {
    let window_label = window.label().to_string();
    let key = (window_label.clone(), subscription_id.clone());

    // Same destroy-race fence as in prepare.
    if app.get_webview_window(&window_label).is_none() {
        let mut subs = state.annotation_subs.lock().await;
        subs.remove(&key);
        return Err("window closed during subscribe".into());
    }

    // Take the receiver out of the slot, leaving the abort guard
    // in place so cleanup still tears the SDK forwarder down.
    let rx = {
        let mut subs = state.annotation_subs.lock().await;
        let slot = subs
            .get_mut(&key)
            .ok_or_else(|| "subscription not found (already torn down?)".to_string())?;
        slot.rx
            .take()
            .ok_or_else(|| "subscription already started".to_string())?
    };

    let event_name = format!("lunaris://annotation-changed/{subscription_id}");
    let target_window = window.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = rx;
        while let Some(change) = rx.recv().await {
            if let Err(e) = target_window.emit(&event_name, &change) {
                log::warn!("annotation pump emit failed: {e}");
            }
        }
    });

    Ok(())
}

/// Tear down a specific subscription. Removing the entry from
/// the map drops the [`crate::SubscriptionSlot`], which aborts
/// the SDK forwarder task (via the held [`os_sdk::AbortOnDrop`])
/// and closes the upstream bus connection.
#[tauri::command]
pub async fn annotation_unsubscribe<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, ShellState>,
    subscription_id: String,
) -> Result<(), String> {
    let key = (window.label().to_string(), subscription_id);
    let mut subs = state.annotation_subs.lock().await;
    subs.remove(&key);
    Ok(())
}
