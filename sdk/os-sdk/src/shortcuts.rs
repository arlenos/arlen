//! `shell.shortcuts` — discoverable quick-actions surfaced in
//! Waypointer when the app is focused.
//!
//! Foundation §6.4 Listing 13, p53. Distinct from the keyboard
//! shortcut registration system in `arlen-input-client` — see
//! `docs/architecture/shortcuts-api.md` for the comparison.
//!
//! Per-app: one shortcut list per app, shared across all the
//! app's windows. Different from toolbar (which is per-window).
//!
//! Wire: `app.shortcut.register` (full-replace),
//! `app.shortcut.state_changed` (diff-update one shortcut),
//! `app.shortcut.cleared`. Action dispatch arrives back via
//! `app.shortcut.action_invoked` with the same shape as
//! `app.toolbar.action_invoked` so the plugin-side action
//! consumer handles both surfaces uniformly.

use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::proto::{
    Shortcut as ProtoShortcut, ShortcutClearedPayload, ShortcutRegisterPayload,
    ShortcutStateChangedPayload,
};

/// Single shortcut entry as registered by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    pub label: String,
    /// ui-kit / Lucide icon identifier.
    pub icon: String,
    /// Opaque action string dispatched back on user click.
    pub action: String,
    /// Tag filter for Focus-Mode-aware rendering. Phase 1
    /// ignores this field entirely; Phase 6 brings tag-aware
    /// filtering once the project tag system lands. Apps
    /// SHOULD declare context tags now so the future filter
    /// turns on without a manifest change.
    #[serde(default)]
    pub context: Vec<String>,
    /// Optional confirmation dialog text. When `Some`, the
    /// shell shows yes/no dialog with this text before
    /// dispatching the click. Cancel = no event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

/// Per-shortcut state diff for [`Shortcuts::set_state`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutState {
    /// `Some(true|false)` to update enabled flag, `None` to
    /// leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// `Some("3")`, `Some("!")` etc. for a small text overlay.
    /// `Some("")` clears the badge. `None` leaves unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
}

/// Surface for the `shell.shortcuts` API.
pub struct Shortcuts<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Shortcuts<E> {
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Register the app's full shortcut list. Replaces any
    /// previously-registered set. Empty `shortcuts` acts as
    /// `clear()`.
    ///
    /// # Errors
    /// [`EmitError::SerializationFailed`] if any shortcut has
    /// an empty or oversized `action` string.
    pub fn register(
        &self,
        shortcuts: Vec<Shortcut>,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        async move {
            // Per-call uniqueness check on `action` strings. The
            // `set_state()` API addresses entries by `action`,
            // so duplicates would make state mutations
            // ambiguous and potentially silent. Foundation
            // §6.4 Listing 13 implicitly assumes uniqueness
            // (each shortcut has its own action label).
            //
            // Codex Sprint-B-fat C10: closes the silent
            // misroute window where `set_state("save")`
            // would land on whichever duplicate the shell
            // happened to look up first.
            let mut seen = std::collections::HashSet::new();
            for s in &shortcuts {
                if s.action.is_empty() {
                    return Err(EmitError::SerializationFailed(
                        "Shortcut.action must not be empty".into(),
                    ));
                }
                if s.action.len() > 256 {
                    return Err(EmitError::SerializationFailed(
                        "Shortcut.action must be <= 256 chars".into(),
                    ));
                }
                if s.label.is_empty() {
                    return Err(EmitError::SerializationFailed(
                        "Shortcut.label must not be empty".into(),
                    ));
                }
                if !seen.insert(s.action.clone()) {
                    return Err(EmitError::SerializationFailed(format!(
                        "duplicate Shortcut.action '{}' in register() — \
                         action strings must be unique within one list",
                        s.action
                    )));
                }
            }

            let payload = ShortcutRegisterPayload {
                app_id: self.app_id.clone(),
                shortcuts: shortcuts.into_iter().map(shortcut_to_proto).collect(),
            };
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("ShortcutRegisterPayload encode is infallible");
            self.emitter.emit("app.shortcut.register", buf).await
        }
    }

    /// Diff-update one shortcut's per-instance state. Action
    /// must reference a previously-registered shortcut; the
    /// shell looks up by `action` and silently no-ops on miss.
    pub fn set_state(
        &self,
        action: impl Into<String>,
        state: ShortcutState,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = ShortcutStateChangedPayload {
            app_id: self.app_id.clone(),
            action: action.into(),
            enabled: state.enabled,
            badge: state.badge,
        };
        async move {
            if payload.action.is_empty() {
                return Err(EmitError::SerializationFailed(
                    "Shortcut.action must not be empty".into(),
                ));
            }
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("ShortcutStateChangedPayload encode is infallible");
            self.emitter.emit("app.shortcut.state_changed", buf).await
        }
    }

    /// Drop every shortcut for this app. Equivalent to
    /// `register([])` but distinct event for audit clarity.
    pub fn clear(&self) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = ShortcutClearedPayload {
            app_id: self.app_id.clone(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("ShortcutClearedPayload encode is infallible");
        async move { self.emitter.emit("app.shortcut.cleared", buf).await }
    }
}

fn shortcut_to_proto(s: Shortcut) -> ProtoShortcut {
    ProtoShortcut {
        label: s.label,
        icon: s.icon,
        action: s.action,
        context: s.context,
        confirm: s.confirm.unwrap_or_default(),
    }
}

/// Decoded form of an `app.shortcut.action_invoked` payload.
/// Same shape as toolbar's [`crate::ActionInvoked`]; kept
/// distinct here for surface-level audit clarity. Apps don't
/// need to differentiate — the plugin-side consumer routes
/// both surfaces to the same `onAction` handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutInvoked {
    pub app_id: String,
    pub action: String,
    pub window_id: String,
}

pub fn decode_shortcut_invoked(payload: &[u8]) -> Option<ShortcutInvoked> {
    use prost::Message;
    crate::proto::ShortcutActionInvokedPayload::decode(payload)
        .ok()
        .map(|p| ShortcutInvoked {
            app_id: p.app_id,
            action: p.action,
            window_id: p.window_id,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    fn decode_register(bytes: &[u8]) -> ShortcutRegisterPayload {
        ShortcutRegisterPayload::decode(bytes).expect("valid payload")
    }
    fn decode_state(bytes: &[u8]) -> ShortcutStateChangedPayload {
        ShortcutStateChangedPayload::decode(bytes).expect("valid payload")
    }

    #[tokio::test]
    async fn register_emits_correct_payload() {
        let emitter = MockEventEmitter::new();
        let sh = Shortcuts::new(emitter.clone(), "com.example.git-app");
        sh.register(vec![
            Shortcut {
                label: "New Issue".into(),
                icon: "plus-circle".into(),
                action: "issue.new".into(),
                context: vec!["project".into(), "git".into()],
                confirm: Some("Create new issue?".into()),
            },
            Shortcut {
                label: "Run Tests".into(),
                icon: "play".into(),
                action: "test.run".into(),
                context: vec![],
                confirm: None,
            },
        ])
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "app.shortcut.register");
        let p = decode_register(&events[0].payload);
        assert_eq!(p.app_id, "com.example.git-app");
        assert_eq!(p.shortcuts.len(), 2);
        assert_eq!(p.shortcuts[0].action, "issue.new");
        assert_eq!(p.shortcuts[0].context, vec!["project", "git"]);
        assert_eq!(p.shortcuts[0].confirm, "Create new issue?");
        assert_eq!(p.shortcuts[1].confirm, "");
    }

    #[tokio::test]
    async fn register_rejects_empty_action() {
        let emitter = MockEventEmitter::new();
        let sh = Shortcuts::new(emitter, "app");
        let err = sh
            .register(vec![Shortcut {
                label: "x".into(),
                icon: "x".into(),
                action: String::new(),
                context: vec![],
                confirm: None,
            }])
            .await
            .unwrap_err();
        assert!(matches!(err, EmitError::SerializationFailed(_)));
    }

    #[tokio::test]
    async fn register_rejects_duplicate_actions() {
        let emitter = MockEventEmitter::new();
        let sh = Shortcuts::new(emitter.clone(), "app");
        let err = sh
            .register(vec![
                Shortcut {
                    label: "Save".into(),
                    icon: "save".into(),
                    action: "file.save".into(),
                    context: vec![],
                    confirm: None,
                },
                Shortcut {
                    label: "Save As".into(),
                    icon: "save".into(),
                    action: "file.save".into(), // duplicate!
                    context: vec![],
                    confirm: None,
                },
            ])
            .await
            .unwrap_err();
        match err {
            EmitError::SerializationFailed(msg) => {
                assert!(
                    msg.contains("duplicate") && msg.contains("file.save"),
                    "error must name the duplicate: {msg}"
                );
            }
            other => panic!("expected SerializationFailed, got {other:?}"),
        }
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn register_rejects_empty_label() {
        let emitter = MockEventEmitter::new();
        let sh = Shortcuts::new(emitter, "app");
        let err = sh
            .register(vec![Shortcut {
                label: String::new(),
                icon: "x".into(),
                action: "x".into(),
                context: vec![],
                confirm: None,
            }])
            .await
            .unwrap_err();
        assert!(matches!(err, EmitError::SerializationFailed(_)));
    }

    #[tokio::test]
    async fn set_state_emits_diff_update() {
        let emitter = MockEventEmitter::new();
        let sh = Shortcuts::new(emitter.clone(), "app");
        sh.set_state(
            "git.push",
            ShortcutState {
                enabled: Some(true),
                badge: Some("3".into()),
            },
        )
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.shortcut.state_changed");
        let p = decode_state(&events[0].payload);
        assert_eq!(p.action, "git.push");
        assert_eq!(p.enabled, Some(true));
        assert_eq!(p.badge, Some("3".to_string()));
    }

    #[tokio::test]
    async fn clear_emits_dedicated_event() {
        let emitter = MockEventEmitter::new();
        let sh = Shortcuts::new(emitter.clone(), "app");
        sh.clear().await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.shortcut.cleared");
    }

    #[test]
    fn decode_shortcut_invoked_round_trips() {
        let payload = crate::proto::ShortcutActionInvokedPayload {
            app_id: "com.example.app".into(),
            action: "git.push".into(),
            window_id: "main".into(),
        };
        let encoded = payload.encode_to_vec();
        let decoded = decode_shortcut_invoked(&encoded).expect("decode");
        assert_eq!(decoded.app_id, "com.example.app");
        assert_eq!(decoded.action, "git.push");
        assert_eq!(decoded.window_id, "main");

        assert!(decode_shortcut_invoked(b"not protobuf").is_none());
    }
}
