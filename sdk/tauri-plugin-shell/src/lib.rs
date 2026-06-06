/// Tauri plugin exposing the Arlen OS `shell.*` API to Tauri apps.
///
/// First-party Tauri apps include this plugin once and immediately have
/// `shell.presence`, `shell.timeline`, and `shell.spatial` available
/// from the TypeScript frontend. The plugin owns the long-lived
/// `UnixEventEmitter` connection to the Event Bus and turns each
/// invocation from the frontend into a typed `os-sdk` call.
///
/// `shell.menu` is **not** exposed by this plugin — that surface lives
/// in `desktop-shell` directly because menus are global state owned by
/// the shell, not per-app state proxied through the Event Bus.
///
/// # Usage (Rust)
///
/// ```rust,ignore
/// fn main() {
///     tauri::Builder::default()
///         .plugin(tauri_plugin_arlen_shell::init())
///         .run(tauri::generate_context!())
///         .expect("error running app");
/// }
/// ```
///
/// # Usage (TypeScript)
///
/// ```typescript
/// import { shell } from "@arlen/tauri-plugin-shell";
///
/// await shell.presence.set({
///   activity: "editing",
///   subject: "report.md",
/// });
/// ```
///
/// # Configuration
///
/// The plugin reads `ARLEN_APP_ID` and the producer-socket env
/// (`ARLEN_PRODUCER_SOCKET`, default
/// `/run/arlen/event-bus-producer.sock`) at init time. Apps that
/// need to override the socket path can do so by setting the env
/// variable before constructing the Tauri builder.

mod commands;

use std::collections::HashMap;
use std::sync::Arc;

use os_sdk::{
    decode_action_invoked, decode_shortcut_invoked, AbortOnDrop, Ambient, AnnotationChange,
    Annotations, Badges, EventConsumer, Presence, Shortcuts, Spatial, Timeline, Toolbar,
    UnixEventConsumer, UnixEventEmitter, UnixGraphClient,
};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, RunEvent, Runtime, WindowEvent,
};
use tokio::sync::{mpsc, Mutex};

/// Key for the per-window subscription map. Composed of the
/// Tauri window label and a stable subscription id chosen by
/// the SDK. Keying by both lets us tear down all subscriptions
/// that belonged to a window when the window is destroyed.
pub type SubscriptionKey = (String, String);

/// Two-phase subscription state.
///
/// `Pending`: backend is connected and the SDK forwarder is
/// pumping into the rx, but no Tauri events are being emitted
/// yet. The frontend has time to register its `listen()` handler
/// after this phase before any events leave the backend.
///
/// `Active`: pump task spawned. Events drain from the buffered
/// rx (which still contains everything that arrived during the
/// pending phase) into per-webview Tauri events.
pub struct SubscriptionSlot {
    pub abort_on_drop: AbortOnDrop,
    pub rx: Option<mpsc::Receiver<AnnotationChange>>,
}

/// Runtime state held by the plugin.
///
/// Each shell.* surface owns its own thin wrapper around the shared
/// `UnixEventEmitter`. The emitter is `Clone` (it just shares an
/// `Arc<Mutex<Option<UnixStream>>>` internally), so cloning per
/// surface is cheap and keeps the surface APIs simple.
pub struct ShellState {
    pub presence: Arc<Presence<UnixEventEmitter>>,
    pub timeline: Arc<Timeline<UnixEventEmitter>>,
    pub spatial: Arc<Spatial<UnixEventEmitter>>,
    pub annotations: Arc<Annotations<UnixEventEmitter, UnixGraphClient>>,
    pub toolbar: Arc<Toolbar<UnixEventEmitter>>,
    pub shortcuts: Arc<Shortcuts<UnixEventEmitter>>,
    pub badges: Arc<Badges<UnixEventEmitter>>,
    pub ambient: Arc<Ambient<UnixEventEmitter>>,
    /// Consumer-side bus client used by annotations on_changed.
    /// Cloned per `subscribe` call (the consumer itself is cheap
    /// to clone; each `subscribe()` opens its own underlying
    /// connection).
    pub consumer: UnixEventConsumer,
    /// Live annotation subscriptions keyed by (window-label,
    /// subscription-id). The slot is in `Pending` between
    /// `prepare` and `start`, then `Active` until cleanup. Drop
    /// of the slot drops the [`AbortOnDrop`] guard which aborts
    /// the SDK forwarder task; if a receiver is still in the
    /// slot (`Pending`) it is dropped along with it.
    pub annotation_subs: Arc<Mutex<HashMap<SubscriptionKey, SubscriptionSlot>>>,
    /// App id used by the toolbar action-invoked filter.
    /// `ARLEN_APP_ID` at plugin init; falls back to "unknown".
    pub app_id: String,
}

impl ShellState {
    fn new() -> Self {
        let producer_socket = std::env::var("ARLEN_PRODUCER_SOCKET")
            .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string());
        let consumer_socket = std::env::var("ARLEN_CONSUMER_SOCKET")
            .unwrap_or_else(|_| "/run/arlen/event-bus-consumer.sock".to_string());
        let daemon_socket = std::env::var("ARLEN_DAEMON_SOCKET")
            .unwrap_or_else(|_| "/run/arlen/knowledge.sock".to_string());
        let app_id =
            std::env::var("ARLEN_APP_ID").unwrap_or_else(|_| "unknown".to_string());

        // One emitter shared across the write-side surfaces; one
        // graph client for annotation reads; one consumer for
        // subscribe-side surfaces.
        let emitter = UnixEventEmitter::new(producer_socket);
        let graph = UnixGraphClient::new(daemon_socket);
        let consumer = UnixEventConsumer::new(consumer_socket);

        Self {
            presence: Arc::new(Presence::new(emitter.clone(), app_id.clone())),
            timeline: Arc::new(Timeline::new(emitter.clone(), app_id.clone())),
            spatial: Arc::new(Spatial::new(emitter.clone(), app_id.clone())),
            toolbar: Arc::new(Toolbar::new(emitter.clone(), app_id.clone())),
            shortcuts: Arc::new(Shortcuts::new(emitter.clone(), app_id.clone())),
            badges: Arc::new(Badges::new(emitter.clone(), app_id.clone())),
            ambient: Arc::new(Ambient::new(emitter.clone(), app_id.clone())),
            annotations: Arc::new(Annotations::new(emitter, graph, app_id.clone())),
            consumer,
            annotation_subs: Arc::new(Mutex::new(HashMap::new())),
            app_id,
        }
    }
}

/// Initialise the Arlen shell plugin.
///
/// Registers all shell.* Tauri commands and constructs the
/// `ShellState` that wraps the Event Bus emitter and consumer.
/// Includes a `RunEvent::WindowEvent::Destroyed` hook that tears
/// down annotation subscriptions belonging to the destroyed
/// window so a webview reload or close cannot leak forwarder
/// tasks (FA E7/E8 in `docs/architecture/annotations-api.md`).
///
/// Apps include the plugin via
/// `Tauri::Builder::plugin(tauri_plugin_arlen_shell::init())`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("arlen-shell")
        .invoke_handler(tauri::generate_handler![
            commands::presence_set,
            commands::presence_clear,
            commands::timeline_record,
            commands::spatial_hint,
            commands::annotation_set,
            commands::annotation_clear,
            commands::annotation_get,
            commands::annotation_subscribe_prepare,
            commands::annotation_subscribe_start,
            commands::annotation_unsubscribe,
            commands::toolbar_set_quick_actions,
            commands::toolbar_set_breadcrumb,
            commands::toolbar_set_progress,
            commands::toolbar_clear_progress,
            commands::toolbar_clear,
            commands::shortcuts_register,
            commands::shortcuts_set_state,
            commands::shortcuts_clear,
            commands::badges_set,
            commands::badges_clear,
            commands::ambient_set,
            commands::ambient_clear,
        ])
        .setup(|app, _api| {
            let state = ShellState::new();
            spawn_action_invoked_consumer(app, &state);
            app.manage(state);
            Ok(())
        })
        .on_event(|app, event| {
            if let RunEvent::WindowEvent {
                label,
                event: WindowEvent::Destroyed,
                ..
            } = event
            {
                cleanup_window(app, label);
            }
        })
        .build()
}

/// Internal helper to keep the action-invoked decode + filter
/// loop typed regardless of which surface fired
/// (toolbar vs. shortcut).
struct ActionTuple {
    app_id: String,
    action: String,
    window_id: String,
}

/// Subscribe to `app.toolbar.action_invoked` and
/// `app.shortcut.action_invoked` Event Bus events and re-emit
/// matching ones (filtered by this app's id) as per-webview
/// `arlen://app-action` Tauri events.
///
/// This is the receive side of the action-dispatch path the
/// desktop-shell pushes when the user clicks a Quick Action or
/// Breadcrumb segment in the TopBar. The shell does not have
/// direct webview handles for other Tauri apps, so it crosses
/// the process boundary via the Event Bus.
///
/// Self-healing: on any subscribe failure or stream end the
/// outer loop reconnects with exponential backoff (capped 30 s).
/// A startup race against the bus or a transient socket error
/// must not permanently disable toolbar dispatch — the inner
/// `EventConsumer::subscribe` only retries the *initial*
/// connect for ~400 ms, so the supervising loop here is what
/// turns "consumer task" into "consumer service".
///
/// Foundation §6.4 Listing 22 + `topbar-toolbar.md` FA6.
fn spawn_action_invoked_consumer<R: Runtime, M: tauri::Manager<R>>(
    app: &M,
    state: &ShellState,
) {
    use tauri::Emitter;

    let consumer = state.consumer.clone();
    let target_app_id = state.app_id.clone();
    let app_handle = app.app_handle().clone();

    tauri::async_runtime::spawn(async move {
        let mut backoff = std::time::Duration::from_millis(500);
        let backoff_max = std::time::Duration::from_secs(30);

        loop {
            let mut rx = match consumer
                .subscribe(vec![
                    "app.toolbar.action_invoked".to_string(),
                    "app.shortcut.action_invoked".to_string(),
                ])
                .await
            {
                Ok(rx) => {
                    backoff = std::time::Duration::from_millis(500);
                    rx
                }
                Err(e) => {
                    log::warn!(
                        "action-invoked subscribe failed (retrying in {:?}): {e}",
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(backoff_max);
                    continue;
                }
            };

            while let Some(event) = rx.recv().await {
                // Decode based on which action surface fired.
                // Both have identical wire shape (app_id, action,
                // window_id) — apps' onAction handler treats
                // them uniformly.
                let invoked = match event.r#type.as_str() {
                    "app.toolbar.action_invoked" => {
                        decode_action_invoked(&event.payload).map(|v| {
                            (v.app_id, v.action, v.window_id)
                        })
                    }
                    "app.shortcut.action_invoked" => {
                        decode_shortcut_invoked(&event.payload).map(|v| {
                            (v.app_id, v.action, v.window_id)
                        })
                    }
                    _ => continue,
                };
                let Some((invoked_app_id, action, window_id)) = invoked else {
                    continue;
                };
                // Re-pack into a struct shape identical to the
                // toolbar branch to keep the rest of the loop
                // simple. `crate::os_sdk::ActionInvoked` would be
                // ideal but it's typed for the toolbar event;
                // we open-code the tuple instead.
                let invoked = ActionTuple {
                    app_id: invoked_app_id,
                    action,
                    window_id,
                };
                // Filter: only forward actions targeted at this app.
                if invoked.app_id != target_app_id {
                    continue;
                }
                // Route per-webview using the window_id field
                // carried in the payload (B8.4 — closes the
                // multi-window same-app routing gap). Falls back
                // to broadcast only when window_id is empty (e.g.
                // legacy producers that pre-date the window_id
                // schema).
                if invoked.window_id.is_empty() {
                    if let Err(e) = app_handle.emit(
                        "arlen://app-action",
                        serde_json::json!({ "action": invoked.action }),
                    ) {
                        log::warn!("toolbar app-action broadcast emit failed: {e}");
                    }
                    continue;
                }
                let Some(window) = app_handle.get_webview_window(&invoked.window_id)
                else {
                    // Window is gone (closed during dispatch).
                    // Drop silently — the action has no recipient.
                    continue;
                };
                if let Err(e) = window.emit(
                    "arlen://app-action",
                    serde_json::json!({ "action": invoked.action }),
                ) {
                    log::warn!("toolbar app-action emit failed: {e}");
                }
            }

            // The receiver yielded None. The SDK consumer's
            // internal reconnect-loop already tried to recover
            // and gave up (channel closed). Retry the entire
            // subscribe from the top with backoff.
            log::warn!("toolbar action-invoked stream ended, resubscribing");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(backoff_max);
        }
    });
}

/// Drop every annotation subscription whose key matches the
/// destroyed window label. Each removed `AbortOnDrop` aborts its
/// SDK forwarder task; the upstream Event Bus connection drops
/// shortly after.
fn cleanup_window<R: Runtime>(app: &tauri::AppHandle<R>, window_label: &str) {
    let Some(state) = app.try_state::<ShellState>() else {
        return;
    };
    let subs = state.annotation_subs.clone();
    let label = window_label.to_string();
    tauri::async_runtime::spawn(async move {
        let mut guard = subs.lock().await;
        guard.retain(|(win, _id), _| win != &label);
    });
}
