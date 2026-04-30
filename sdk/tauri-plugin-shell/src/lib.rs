/// Tauri plugin exposing the Lunaris OS `shell.*` API to Tauri apps.
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
///         .plugin(tauri_plugin_lunaris_shell::init())
///         .run(tauri::generate_context!())
///         .expect("error running app");
/// }
/// ```
///
/// # Usage (TypeScript)
///
/// ```typescript
/// import { shell } from "@lunaris/tauri-plugin-shell";
///
/// await shell.presence.set({
///   activity: "editing",
///   subject: "report.md",
/// });
/// ```
///
/// # Configuration
///
/// The plugin reads `LUNARIS_APP_ID` and the producer-socket env
/// (`LUNARIS_PRODUCER_SOCKET`, default
/// `/run/lunaris/event-bus-producer.sock`) at init time. Apps that
/// need to override the socket path can do so by setting the env
/// variable before constructing the Tauri builder.

mod commands;

use std::collections::HashMap;
use std::sync::Arc;

use os_sdk::{
    AbortOnDrop, AnnotationChange, Annotations, Presence, Spatial, Timeline,
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
}

impl ShellState {
    fn new() -> Self {
        let producer_socket = std::env::var("LUNARIS_PRODUCER_SOCKET")
            .unwrap_or_else(|_| "/run/lunaris/event-bus-producer.sock".to_string());
        let consumer_socket = std::env::var("LUNARIS_CONSUMER_SOCKET")
            .unwrap_or_else(|_| "/run/lunaris/event-bus-consumer.sock".to_string());
        let daemon_socket = std::env::var("LUNARIS_DAEMON_SOCKET")
            .unwrap_or_else(|_| "/run/lunaris/knowledge.sock".to_string());
        let app_id =
            std::env::var("LUNARIS_APP_ID").unwrap_or_else(|_| "unknown".to_string());

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
            annotations: Arc::new(Annotations::new(emitter, graph, app_id)),
            consumer,
            annotation_subs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Initialise the Lunaris shell plugin.
///
/// Registers all shell.* Tauri commands and constructs the
/// `ShellState` that wraps the Event Bus emitter and consumer.
/// Includes a `RunEvent::WindowEvent::Destroyed` hook that tears
/// down annotation subscriptions belonging to the destroyed
/// window so a webview reload or close cannot leak forwarder
/// tasks (FA E7/E8 in `docs/architecture/annotations-api.md`).
///
/// Apps include the plugin via
/// `Tauri::Builder::plugin(tauri_plugin_lunaris_shell::init())`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("lunaris-shell")
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
        ])
        .setup(|app, _api| {
            app.manage(ShellState::new());
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
