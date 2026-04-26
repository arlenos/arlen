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

use std::sync::Arc;

use os_sdk::{Annotations, Presence, Spatial, Timeline, UnixEventEmitter, UnixGraphClient};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

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
}

impl ShellState {
    fn new() -> Self {
        let producer_socket = std::env::var("LUNARIS_PRODUCER_SOCKET")
            .unwrap_or_else(|_| "/run/lunaris/event-bus-producer.sock".to_string());
        let daemon_socket = std::env::var("LUNARIS_DAEMON_SOCKET")
            .unwrap_or_else(|_| "/run/lunaris/knowledge.sock".to_string());
        let app_id =
            std::env::var("LUNARIS_APP_ID").unwrap_or_else(|_| "unknown".to_string());

        // One emitter shared across the write-side surfaces; one
        // graph client for annotation reads.
        let emitter = UnixEventEmitter::new(producer_socket);
        let graph = UnixGraphClient::new(daemon_socket);

        Self {
            presence: Arc::new(Presence::new(emitter.clone(), app_id.clone())),
            timeline: Arc::new(Timeline::new(emitter.clone(), app_id.clone())),
            spatial: Arc::new(Spatial::new(emitter.clone(), app_id.clone())),
            annotations: Arc::new(Annotations::new(emitter, graph, app_id)),
        }
    }
}

/// Initialise the Lunaris shell plugin.
///
/// Registers the four shell.* Tauri commands and constructs the
/// `ShellState` that wraps the Event Bus emitter. Apps include
/// the plugin via `Tauri::Builder::plugin(tauri_plugin_lunaris_shell::init())`.
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
        ])
        .setup(|app, _api| {
            app.manage(ShellState::new());
            Ok(())
        })
        .build()
}
