//! `shell.toolbar` — first-party app surface for pushing
//! Quick Actions, Breadcrumb, or Progress into the top bar's
//! `slot-toolbar`.
//!
//! Foundation §6.4 Listings 22-24. The three sub-surfaces are
//! mutually exclusive in the same slot — setting one clears
//! the others on the shell side. Auto-clear on focus loss is
//! shell-driven (per-app state survives blur, render disappears
//! when another app gets focus).
//!
//! Event Bus carrier: `app.toolbar.quick_actions`,
//! `app.toolbar.breadcrumb`, `app.toolbar.progress`,
//! `app.toolbar.progress_cleared`, `app.toolbar.cleared`. See
//! `docs/architecture/topbar-toolbar.md` for the full contract.
//!
//! The shell dispatches Quick-Action / Breadcrumb clicks back
//! to the source app via Tauri events scoped to the focused
//! webview — frontend code subscribes via the Tauri-plugin's
//! `onAction` helper. The Rust SDK only owns the producer side.

use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::proto::{
    ToolbarBreadcrumbItem as ProtoBreadcrumbItem, ToolbarBreadcrumbPayload,
    ToolbarClearedPayload, ToolbarProgressClearedPayload, ToolbarProgressPayload,
    ToolbarQuickAction as ProtoQuickAction, ToolbarQuickActionsPayload,
};

/// Hard cap from foundation §6.4 Listing 22. The SDK rejects
/// emits with more than this; the shell renders at most this
/// many even if a non-SDK producer pushes more.
pub const MAX_QUICK_ACTIONS: usize = 3;

/// Single Quick Action button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickAction {
    /// ui-kit / Lucide icon identifier.
    pub icon: String,
    /// Opaque action string dispatched back to the app on
    /// click. App-defined namespace.
    pub action: String,
    pub tooltip: String,
    /// `Some(true)` makes the button render as a toggle; pair
    /// with `active` to set its checked state.
    #[serde(default)]
    pub toggle: Option<bool>,
    #[serde(default)]
    pub active: Option<bool>,
}

/// Single Breadcrumb segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreadcrumbItem {
    pub label: String,
    pub action: String,
}

/// Progress indicator state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressState {
    /// Clamped to [0.0, 1.0] before emit.
    pub value: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Surface for the `shell.toolbar` API.
///
/// One Toolbar per app process. Every emit takes a `window_id`
/// (Tauri webview label) so the shell can key state per-(app,
/// window) and route action dispatch back to the originating
/// webview specifically. Multi-window apps that want distinct
/// toolbars in different windows pass different `window_id`
/// values per call. Single-window or non-Tauri callers may
/// pass the empty string (legacy / broadcast mode — the shell
/// then fans the dispatched action out to all webviews of the
/// app).
pub struct Toolbar<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Toolbar<E> {
    /// Create a new toolbar surface bound to a specific emitter
    /// and app id.
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Push Quick Action buttons into the toolbar slot. Replaces
    /// any previously-set Quick Actions, Breadcrumb, or Progress
    /// on the shell side (mutually exclusive).
    ///
    /// # Errors
    /// [`EmitError::SerializationFailed`] if `actions.len() >
    /// MAX_QUICK_ACTIONS` (foundation §6.4 Listing 22), or the
    /// underlying emitter cannot reach the bus.
    pub fn set_quick_actions<'a>(
        &'a self,
        window_id: impl Into<String> + Send + 'a,
        actions: Vec<QuickAction>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        let window_id = window_id.into();
        async move {
            if actions.len() > MAX_QUICK_ACTIONS {
                return Err(EmitError::SerializationFailed(format!(
                    "shell.toolbar.setQuickActions: max {MAX_QUICK_ACTIONS} actions, \
                     got {}",
                    actions.len()
                )));
            }
            for a in &actions {
                if a.action.is_empty() {
                    return Err(EmitError::SerializationFailed(
                        "QuickAction.action must not be empty".into(),
                    ));
                }
                if a.action.len() > 256 {
                    return Err(EmitError::SerializationFailed(
                        "QuickAction.action must be <= 256 chars".into(),
                    ));
                }
            }

            let payload = ToolbarQuickActionsPayload {
                app_id: self.app_id.clone(),
                actions: actions.into_iter().map(quick_action_to_proto).collect(),
                window_id,
            };
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("ToolbarQuickActionsPayload encode is infallible");
            self.emitter.emit("app.toolbar.quick_actions", buf).await
        }
    }

    /// Push a Breadcrumb path. Replaces any previously-set
    /// toolbar variant.
    pub fn set_breadcrumb<'a>(
        &'a self,
        window_id: impl Into<String> + Send + 'a,
        items: Vec<BreadcrumbItem>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        let window_id = window_id.into();
        async move {
            for item in &items {
                if item.action.is_empty() {
                    return Err(EmitError::SerializationFailed(
                        "BreadcrumbItem.action must not be empty".into(),
                    ));
                }
                if item.action.len() > 256 {
                    return Err(EmitError::SerializationFailed(
                        "BreadcrumbItem.action must be <= 256 chars".into(),
                    ));
                }
            }

            let payload = ToolbarBreadcrumbPayload {
                app_id: self.app_id.clone(),
                items: items.into_iter().map(breadcrumb_item_to_proto).collect(),
                window_id,
            };
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("ToolbarBreadcrumbPayload encode is infallible");
            self.emitter.emit("app.toolbar.breadcrumb", buf).await
        }
    }

    /// Set the Progress indicator. Replaces any previously-set
    /// toolbar variant. `value` is clamped to `[0.0, 1.0]`;
    /// non-finite inputs (`NaN`, `Infinity`) are rejected
    /// rather than silently coerced because `f32::clamp`
    /// propagates `NaN` and a `NaN` width in the renderer is
    /// undefined behaviour from the user's perspective.
    pub fn set_progress<'a>(
        &'a self,
        window_id: impl Into<String> + Send + 'a,
        progress: ProgressState,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        let window_id = window_id.into();
        async move {
            if !progress.value.is_finite() {
                return Err(EmitError::SerializationFailed(format!(
                    "ProgressState.value must be finite, got {}",
                    progress.value
                )));
            }
            let clamped = progress.value.clamp(0.0, 1.0);
            let payload = ToolbarProgressPayload {
                app_id: self.app_id.clone(),
                value: clamped,
                label: progress.label.unwrap_or_default(),
                window_id,
            };
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("ToolbarProgressPayload encode is infallible");
            self.emitter.emit("app.toolbar.progress", buf).await
        }
    }

    /// Clear only the Progress slot. Quick Actions or Breadcrumb
    /// state set previously is unaffected. (Use `clear()` for
    /// the broad sweep.)
    pub fn clear_progress<'a>(
        &'a self,
        window_id: impl Into<String> + Send + 'a,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        let payload = ToolbarProgressClearedPayload {
            app_id: self.app_id.clone(),
            window_id: window_id.into(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("ToolbarProgressClearedPayload encode is infallible");
        async move {
            self.emitter
                .emit("app.toolbar.progress_cleared", buf)
                .await
        }
    }

    /// Drop every toolbar variant for this app. Shell-side
    /// auto-clear on focus loss makes this optional, but apps
    /// that want eager clear-on-blur (e.g. mid-task abort) call
    /// this explicitly.
    pub fn clear<'a>(
        &'a self,
        window_id: impl Into<String> + Send + 'a,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + 'a {
        let payload = ToolbarClearedPayload {
            app_id: self.app_id.clone(),
            window_id: window_id.into(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("ToolbarClearedPayload encode is infallible");
        async move { self.emitter.emit("app.toolbar.cleared", buf).await }
    }
}

/// Decoded form of an `app.toolbar.action_invoked` Event Bus
/// payload — the back-channel the desktop-shell uses to push
/// Quick-Action / Breadcrumb clicks across the process boundary
/// to the source app's specific window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionInvoked {
    pub app_id: String,
    pub action: String,
    /// Tauri webview label of the originating window. Empty
    /// for legacy producers; consumers should fall back to
    /// app-wide broadcast in that case.
    pub window_id: String,
}

/// Decode an `app.toolbar.action_invoked` payload. Used by
/// consumer-side code (e.g. tauri-plugin-shell's plugin-init
/// consumer) so the proto module can stay `pub(crate)`. Returns
/// `None` for malformed payloads — callers skip silently.
pub fn decode_action_invoked(payload: &[u8]) -> Option<ActionInvoked> {
    use prost::Message;
    crate::proto::ToolbarActionInvokedPayload::decode(payload)
        .ok()
        .map(|p| ActionInvoked {
            app_id: p.app_id,
            action: p.action,
            window_id: p.window_id,
        })
}

fn quick_action_to_proto(a: QuickAction) -> ProtoQuickAction {
    ProtoQuickAction {
        icon: a.icon,
        action: a.action,
        tooltip: a.tooltip,
        toggle: a.toggle.unwrap_or(false),
        active: a.active.unwrap_or(false),
    }
}

fn breadcrumb_item_to_proto(i: BreadcrumbItem) -> ProtoBreadcrumbItem {
    ProtoBreadcrumbItem {
        label: i.label,
        action: i.action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    fn decode_quick(bytes: &[u8]) -> ToolbarQuickActionsPayload {
        ToolbarQuickActionsPayload::decode(bytes).expect("valid payload")
    }
    fn decode_breadcrumb(bytes: &[u8]) -> ToolbarBreadcrumbPayload {
        ToolbarBreadcrumbPayload::decode(bytes).expect("valid payload")
    }
    fn decode_progress(bytes: &[u8]) -> ToolbarProgressPayload {
        ToolbarProgressPayload::decode(bytes).expect("valid payload")
    }

    #[tokio::test]
    async fn set_quick_actions_emits_correct_event() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "com.example.editor");
        tb.set_quick_actions("main", vec![
            QuickAction {
                icon: "share".into(),
                action: "file.share".into(),
                tooltip: "Share".into(),
                toggle: None,
                active: None,
            },
            QuickAction {
                icon: "star".into(),
                action: "file.bookmark".into(),
                tooltip: "Bookmark".into(),
                toggle: Some(true),
                active: Some(false),
            },
        ])
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.toolbar.quick_actions");
        let p = decode_quick(&events[0].payload);
        assert_eq!(p.app_id, "com.example.editor");
        assert_eq!(p.window_id, "main");
        assert_eq!(p.actions.len(), 2);
        assert_eq!(p.actions[1].action, "file.bookmark");
        assert!(p.actions[1].toggle);
    }

    #[tokio::test]
    async fn set_quick_actions_rejects_more_than_max() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "app");
        let too_many: Vec<QuickAction> = (0..4)
            .map(|i| QuickAction {
                icon: "x".into(),
                action: format!("a{i}"),
                tooltip: String::new(),
                toggle: None,
                active: None,
            })
            .collect();
        let err = tb.set_quick_actions("main", too_many).await.unwrap_err();
        match err {
            EmitError::SerializationFailed(msg) => {
                assert!(msg.contains("max 3"));
            }
            other => panic!("expected SerializationFailed, got {other:?}"),
        }
        // No event emitted on validation error.
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn set_quick_actions_rejects_empty_action() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter, "app");
        let err = tb
            .set_quick_actions("main", vec![QuickAction {
                icon: "x".into(),
                action: String::new(),
                tooltip: String::new(),
                toggle: None,
                active: None,
            }])
            .await
            .unwrap_err();
        assert!(matches!(err, EmitError::SerializationFailed(_)));
    }

    #[tokio::test]
    async fn set_breadcrumb_emits_correct_event() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "com.example.files");
        tb.set_breadcrumb("main", vec![
            BreadcrumbItem {
                label: "Home".into(),
                action: "nav.home".into(),
            },
            BreadcrumbItem {
                label: "Projects".into(),
                action: "nav.projects".into(),
            },
        ])
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.toolbar.breadcrumb");
        let p = decode_breadcrumb(&events[0].payload);
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.items[0].label, "Home");
    }

    #[tokio::test]
    async fn set_progress_clamps_to_unit_interval() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "app");
        tb.set_progress("main", ProgressState {
            value: 1.5,
            label: Some("Compiling".into()),
        })
        .await
        .unwrap();
        tb.set_progress("main", ProgressState {
            value: -0.5,
            label: None,
        })
        .await
        .unwrap();

        let events = emitter.emitted().await;
        let high = decode_progress(&events[0].payload);
        let low = decode_progress(&events[1].payload);
        assert!((high.value - 1.0).abs() < f32::EPSILON);
        assert!(low.value.abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn set_progress_rejects_nan_and_infinity() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "app");

        let nan_err = tb
            .set_progress("main", ProgressState {
                value: f32::NAN,
                label: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(nan_err, EmitError::SerializationFailed(_)));

        let inf_err = tb
            .set_progress("main", ProgressState {
                value: f32::INFINITY,
                label: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(inf_err, EmitError::SerializationFailed(_)));

        let neg_inf_err = tb
            .set_progress("main", ProgressState {
                value: f32::NEG_INFINITY,
                label: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(neg_inf_err, EmitError::SerializationFailed(_)));

        // No event emitted on validation error.
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn clear_progress_emits_dedicated_event() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "app");
        tb.clear_progress("main").await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.toolbar.progress_cleared");
    }

    #[tokio::test]
    async fn clear_emits_blanket_cleared_event() {
        let emitter = MockEventEmitter::new();
        let tb = Toolbar::new(emitter.clone(), "app");
        tb.clear("main").await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.toolbar.cleared");
    }
}
