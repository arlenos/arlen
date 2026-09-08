//! Tauri plugin exposing the Arlen OS `shell.*` API to Tauri apps.
//!
//! First-party Tauri apps include this plugin once and immediately have
//! `shell.presence`, `shell.timeline`, and `shell.spatial` available
//! from the TypeScript frontend. The plugin owns the long-lived
//! `UnixEventEmitter` connection to the Event Bus and turns each
//! invocation from the frontend into a typed `os-sdk` call.
//!
//! `shell.menu` is **not** exposed by this plugin — that surface lives
//! in `desktop-shell` directly because menus are global state owned by
//! the shell, not per-app state proxied through the Event Bus.
//!
//! # Usage (Rust)
//!
//! ```rust,ignore
//! fn main() {
//!     tauri::Builder::default()
//!         .plugin(tauri_plugin_arlen_shell::init())
//!         .run(tauri::generate_context!())
//!         .expect("error running app");
//! }
//! ```
//!
//! # Usage (TypeScript)
//!
//! ```typescript
//! import { shell } from "@arlen/tauri-plugin-shell";
//!
//! await shell.presence.set({
//!   activity: "editing",
//!   subject: "report.md",
//! });
//! ```
//!
//! # Configuration
//!
//! The plugin reads `ARLEN_APP_ID` and the producer-socket env
//! (`ARLEN_PRODUCER_SOCKET`, default
//! `/run/arlen/event-bus-producer.sock`) at init time. Apps that
//! need to override the socket path can do so by setting the env
//! variable before constructing the Tauri builder.

mod commands;
pub mod locale;
pub mod print;
pub mod theme;

use std::collections::HashMap;
use std::sync::Arc;

use os_sdk::{
    decode_action_invoked, decode_shortcut_invoked, AbortOnDrop, Ambient, AnnotationChange,
    Annotations, Badges, EventConsumer, Presence, Shortcuts, Spatial, Timeline, Toolbar,
    Menu, UnixEventConsumer, UnixEventEmitter, UnixGraphClient,
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
    /// The app's global menu, rendered by the shell's topbar while one of the
    /// app's windows is focused.
    pub menu: Arc<Menu<UnixEventEmitter>>,
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
    /// The app's bus identity - the toolbar/menu correlation key the shell
    /// matches against the focused window's app_id. `ARLEN_APP_ID` if set, else
    /// the Tauri bundle identifier; never the old "unknown", which matched no
    /// focused window so an app's toolbar/menu never appeared (PR-4).
    pub app_id: String,
}

impl ShellState {
    fn new(default_app_id: &str) -> Self {
        let producer_socket = std::env::var("ARLEN_PRODUCER_SOCKET")
            .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string());
        let consumer_socket = std::env::var("ARLEN_CONSUMER_SOCKET")
            .unwrap_or_else(|_| "/run/arlen/event-bus-consumer.sock".to_string());
        let daemon_socket = std::env::var("ARLEN_DAEMON_SOCKET")
            .unwrap_or_else(|_| "/run/arlen/knowledge.sock".to_string());
        // The app's bus identity. `ARLEN_APP_ID` wins (a launcher can pin it);
        // otherwise the Tauri bundle identifier, which is the app's Wayland
        // toplevel app_id the shell correlates the toolbar/menu slot against. The
        // old "unknown" default matched no focused window, so an app's toolbar
        // never appeared (PR-4).
        let app_id = std::env::var("ARLEN_APP_ID").unwrap_or_else(|_| {
            if default_app_id.is_empty() {
                "unknown".to_string()
            } else {
                default_app_id.to_string()
            }
        });

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
            menu: Arc::new(Menu::new(emitter.clone(), app_id.clone())),
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
            commands::menu_register,
            commands::menu_unregister,
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
            theme::theme_get,
            locale::locale_get,
            print::print_file,
        ])
        .setup(|app, _api| {
            // Default the bus identity to the app's bundle identifier (the
            // Wayland toplevel app_id), so the shell can correlate this app's
            // toolbar/menu to its focused window (PR-4).
            let state = ShellState::new(&app.config().identifier);
            spawn_action_invoked_consumer(app, &state);
            app.manage(state);
            // Watch the shell's theme broadcast so this app live-reskins on a
            // theme switch (GAP-20). The frontend `initArlenTheme()` primitive
            // injects the initial theme via `theme_get` + listens for
            // `arlen://theme-v2-changed`, which this watcher emits.
            theme::spawn_theme_watcher(app.app_handle().clone());
            // Same shape for the language: read once, then follow the file.
            locale::spawn_locale_watcher(app.app_handle().clone());
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

/// What one bus event on the three action topics becomes for this app.
///
/// Split out of the consumer loop so the ROUTING is testable without a bus, a
/// runtime or a window: which topic goes to which Tauri event, whose clicks are
/// kept, and what an undecodable payload does. The loop below is then delivery
/// and nothing else.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Relay {
    /// A toolbar or shortcut click. Routed to `window_id`, or broadcast when it
    /// is empty (a producer that pre-dates the window_id schema).
    AppAction { action: String, window_id: String },
    /// A top-bar menu click. Per-app rather than per-window, so no routing.
    MenuAction { app_id: String, action: String },
}

/// Decide what an event becomes, or nothing.
///
/// Nothing covers three cases and they are all deliberate: a topic this app does
/// not relay, a payload that will not decode, and a click aimed at another app -
/// every subscriber on the bus sees every other app's clicks, so the id filter
/// is what makes this a per-app channel rather than a broadcast one.
pub(crate) fn relay_for(topic: &str, payload: &[u8], app_id: &str) -> Option<Relay> {
    match topic {
        "app.menu.action_invoked" => {
            let v = decode_shortcut_invoked(payload)?;
            (v.app_id == app_id).then(|| Relay::MenuAction {
                app_id: v.app_id,
                action: v.action,
            })
        }
        "app.toolbar.action_invoked" => {
            let v = decode_action_invoked(payload)?;
            (v.app_id == app_id).then(|| Relay::AppAction {
                action: v.action,
                window_id: v.window_id,
            })
        }
        "app.shortcut.action_invoked" => {
            let v = decode_shortcut_invoked(payload)?;
            (v.app_id == app_id).then(|| Relay::AppAction {
                action: v.action,
                window_id: v.window_id,
            })
        }
        _ => None,
    }
}

/// Subscribe to the three action topics an app hears back on and re-emit the
/// ones aimed at it: toolbar and shortcut clicks as `arlen://app-action`, menu
/// clicks as `arlen://menu-action`.
///
/// THE MENU TOPIC WAS MISSING AND ELEVEN APPS WERE LISTENING FOR IT. An app
/// publishes its top-bar menu through this plugin, the shell draws it, the user
/// picks an item, and the shell pushes the action back onto the bus as
/// `app.menu.action_invoked`. Two apps wrote their own consumer for that; the
/// rest listened for a relay nobody had built, so their menus were drawn and
/// inert. It belongs here for the same reason the other two topics do: the
/// plugin is already the one thing running inside every app's process with a
/// bus connection.
///
/// It stays a SEPARATE Tauri event rather than joining `arlen://app-action`,
/// because the apps already listen for `arlen://menu-action` with a payload
/// carrying `app_id`, and a menu click is a different surface from a toolbar
/// button even where the handler is the same.
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
                    "app.menu.action_invoked".to_string(),
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
                let Some(relay) = relay_for(&event.r#type, &event.payload, &target_app_id)
                else {
                    continue;
                };
                match relay {
                    Relay::MenuAction { app_id, action } => {
                        // The superset payload: every listener destructures
                        // `{app_id, action}` and filters on the id itself.
                        if let Err(e) = app_handle.emit(
                            "arlen://menu-action",
                            serde_json::json!({ "app_id": app_id, "action": action }),
                        ) {
                            log::warn!("menu-action emit failed: {e}");
                        }
                    }
                    Relay::AppAction { action, window_id } => {
                        // Route per-webview using the window_id the payload
                        // carries (B8.4 - closes the multi-window same-app
                        // routing gap). Broadcast only when it is empty.
                        if window_id.is_empty() {
                            if let Err(e) = app_handle.emit(
                                "arlen://app-action",
                                serde_json::json!({ "action": action }),
                            ) {
                                log::warn!("toolbar app-action broadcast emit failed: {e}");
                            }
                            continue;
                        }
                        let Some(window) = app_handle.get_webview_window(&window_id) else {
                            // Window is gone (closed during dispatch). Drop
                            // silently - the action has no recipient.
                            continue;
                        };
                        if let Err(e) = window.emit(
                            "arlen://app-action",
                            serde_json::json!({ "action": action }),
                        ) {
                            log::warn!("toolbar app-action emit failed: {e}");
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::{relay_for, Relay};
    use prost::Message as _;

    /// The wire shape the shell publishes for a menu or shortcut click.
    fn shortcut_payload(app_id: &str, action: &str, window_id: &str) -> Vec<u8> {
        os_sdk::proto::ShortcutActionInvokedPayload {
            app_id: app_id.into(),
            action: action.into(),
            window_id: window_id.into(),
        }
        .encode_to_vec()
    }

    /// The toolbar's own payload, which is a different message on the wire.
    fn toolbar_payload(app_id: &str, action: &str, window_id: &str) -> Vec<u8> {
        os_sdk::proto::ToolbarActionInvokedPayload {
            app_id: app_id.into(),
            action: action.into(),
            window_id: window_id.into(),
        }
        .encode_to_vec()
    }

    #[test]
    fn a_menu_click_becomes_a_menu_action_carrying_its_app_id() {
        // The listeners in eleven apps destructure `{app_id, action}` and filter
        // on the id, so the id has to survive the relay.
        let relay = relay_for(
            "app.menu.action_invoked",
            &shortcut_payload("dev.arlen.clock", "alarm.new", ""),
            "dev.arlen.clock",
        );
        assert_eq!(
            relay,
            Some(Relay::MenuAction {
                app_id: "dev.arlen.clock".into(),
                action: "alarm.new".into(),
            })
        );
    }

    #[test]
    fn a_toolbar_click_keeps_its_window() {
        let relay = relay_for(
            "app.toolbar.action_invoked",
            &toolbar_payload("dev.arlen.files", "view.refresh", "main"),
            "dev.arlen.files",
        );
        assert_eq!(
            relay,
            Some(Relay::AppAction {
                action: "view.refresh".into(),
                window_id: "main".into(),
            })
        );
    }

    #[test]
    fn a_shortcut_click_is_an_app_action_too() {
        let relay = relay_for(
            "app.shortcut.action_invoked",
            &shortcut_payload("dev.arlen.files", "search", ""),
            "dev.arlen.files",
        );
        assert!(matches!(relay, Some(Relay::AppAction { .. })));
    }

    #[test]
    fn another_apps_click_is_not_this_apps_business() {
        // Every subscriber sees every app's clicks. The id filter is what makes
        // this a per-app channel, and dropping it would put one app's menu
        // actions into every other app's webview.
        for topic in [
            "app.menu.action_invoked",
            "app.shortcut.action_invoked",
            "app.toolbar.action_invoked",
        ] {
            let payload = if topic == "app.toolbar.action_invoked" {
                toolbar_payload("dev.arlen.mail", "compose", "")
            } else {
                shortcut_payload("dev.arlen.mail", "compose", "")
            };
            assert_eq!(relay_for(topic, &payload, "dev.arlen.clock"), None, "{topic}");
        }
    }

    #[test]
    fn a_topic_this_app_does_not_relay_is_ignored() {
        assert_eq!(
            relay_for(
                "app.menu.registered",
                &shortcut_payload("dev.arlen.clock", "alarm.new", ""),
                "dev.arlen.clock",
            ),
            None
        );
    }

    #[test]
    fn a_payload_that_will_not_decode_relays_nothing() {
        // Fail quiet rather than guess: an undecodable payload is a producer
        // this build does not understand, and inventing an action from it would
        // run something nobody asked for.
        assert_eq!(
            relay_for("app.menu.action_invoked", b"not a protobuf at all", "dev.arlen.clock"),
            None
        );
    }
}
