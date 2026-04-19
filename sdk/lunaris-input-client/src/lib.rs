//! Client for the Lunaris compositor's input services.
//!
//! Two D-Bus interfaces on the session bus:
//!
//! * `org.lunaris.App1` at `/org/lunaris/App` — apps call
//!   [`AppClient::register`] once at startup to declare their identity
//!   and the action ids they accept. The compositor uses this to
//!   correlate focused Wayland toplevels with D-Bus registrations.
//! * `org.lunaris.InputManager1` at `/org/lunaris/InputManager1` —
//!   apps call [`InputManagerClient::register_binding`] to bind a
//!   keystroke, and subscribe to the `BindingInvoked` signal via
//!   [`InputManagerClient::listen`] to react when the compositor
//!   forwards a dispatched keypress.
//!
//! Typical app integration:
//!
//! ```no_run
//! use lunaris_input_client::{AppClient, InputManagerClient, DeclaredAction};
//!
//! # async fn example() -> zbus::Result<()> {
//! let conn = zbus::Connection::session().await?;
//!
//! // Step 1: identify the app so the compositor can route focused
//! // shortcuts back to us.
//! let app = AppClient::new(&conn).await?;
//! app.register(
//!     "com.example.editor",
//!     "Example Editor",
//!     vec![DeclaredAction {
//!         id: "save".into(),
//!         label: "Save".into(),
//!         description: String::new(),
//!     }],
//!     vec!["register_focused_bindings".into()],
//! ).await?;
//!
//! // Step 2: bind Ctrl+S to the `save` action while our window is focused.
//! let input = InputManagerClient::new(&conn).await?;
//! let result = input
//!     .register_binding("Ctrl+S", "save", "app_focused", "com.example.editor")
//!     .await?;
//! assert!(result.success);
//!
//! // Step 3: act on invocations.
//! let stream = input.listen().await?;
//! futures_util::pin_mut!(stream);
//! use futures_util::StreamExt;
//! while let Some((binding, action)) = stream.next().await {
//!     println!("pressed {binding}: run {action}");
//! }
//! # Ok(()) }
//! ```
//!
//! The compositor's `NameOwnerChanged` cleanup removes both the
//! registration and any owned bindings automatically when the process
//! exits — explicit unregister calls are optional.

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

pub use zbus;

const APP_SERVICE: &str = "org.lunaris.App1";
const APP_PATH: &str = "/org/lunaris/App";
const INPUT_SERVICE: &str = "org.lunaris.InputManager1";
const INPUT_PATH: &str = "/org/lunaris/InputManager1";

// ---------------------------------------------------------------------------
// Wire types (mirror of compositor/src/dbus/*.rs)
// ---------------------------------------------------------------------------

/// An action the app exposes to Settings for rebinding. Sent with
/// [`AppClient::register`]; echoed back in the UI so users can rebind
/// actions even when the app is not running.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DeclaredAction {
    pub id: String,
    pub label: String,
    /// Pass the empty string when absent — the compositor's interface
    /// rejects `Option<String>` in method signatures because some
    /// client bindings struggle with `Maybe<s>` marshalling.
    pub description: String,
}

/// Detail about a binding that already exists when an app tries to
/// register a colliding one. Returned inside [`RegisterResult`].
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ConflictInfo {
    pub binding: String,
    pub existing_action: String,
    pub existing_scope: String,
    pub existing_owner: String,
}

/// Result of [`InputManagerClient::register_binding`]. A flat struct
/// (instead of `Result<(), ConflictInfo>`) because zbus methods can
/// only carry one out-tuple; this lets callers discriminate on
/// `success` without catching a D-Bus error.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RegisterResult {
    pub success: bool,
    pub conflict: Vec<ConflictInfo>,
}

// ---------------------------------------------------------------------------
// org.lunaris.App1 proxy
// ---------------------------------------------------------------------------

#[zbus::proxy(
    interface = "org.lunaris.App1",
    default_service = "org.lunaris.App1",
    default_path = "/org/lunaris/App"
)]
trait AppBus {
    fn register_app(
        &self,
        app_id: &str,
        name: &str,
        actions: Vec<DeclaredAction>,
        permissions: Vec<String>,
    ) -> zbus::Result<bool>;

    fn unregister_app(&self) -> zbus::Result<bool>;

    fn get_app_id(&self, owner: &str) -> zbus::Result<String>;

    fn list_apps(&self) -> zbus::Result<Vec<(String, String)>>;
}

/// Convenience wrapper around the generated [`AppBusProxy`].
pub struct AppClient<'c> {
    proxy: AppBusProxy<'c>,
}

impl<'c> AppClient<'c> {
    /// Connect to `org.lunaris.App1` on the session bus.
    pub async fn new(conn: &zbus::Connection) -> zbus::Result<Self> {
        let proxy = AppBusProxy::builder(conn)
            .destination(APP_SERVICE)?
            .path(APP_PATH)?
            .build()
            .await?;
        Ok(Self { proxy })
    }

    /// Register or update this process as the authoritative D-Bus
    /// client for `app_id`. Calling this twice overwrites the
    /// previous entry — safe to use for updating the declared action
    /// list without reconnecting.
    ///
    /// `permissions` are strings the compositor recognises to gate
    /// sensitive scopes. Known values:
    ///
    /// * `"register_focused_bindings"` — required for the
    ///   `app_focused` scope on `RegisterBinding`.
    /// * `"register_global_bindings"` — required for the `app_global`
    ///   scope. Usually reserved for first-party apps.
    ///
    /// Unknown strings are preserved on the compositor side and may
    /// be acted on by future versions; pass an empty vec if you do
    /// not need any gated scope.
    pub async fn register(
        &self,
        app_id: &str,
        name: &str,
        actions: Vec<DeclaredAction>,
        permissions: Vec<String>,
    ) -> zbus::Result<bool> {
        self.proxy
            .register_app(app_id, name, actions, permissions)
            .await
    }

    /// Explicitly unregister. Optional — the compositor drops stale
    /// registrations via `NameOwnerChanged` when the process exits.
    pub async fn unregister(&self) -> zbus::Result<bool> {
        self.proxy.unregister_app().await
    }

    /// Look up the `app_id` registered under a given D-Bus unique
    /// name. Returns the empty string if nothing is registered.
    pub async fn app_id_for(&self, owner: &str) -> zbus::Result<String> {
        self.proxy.get_app_id(owner).await
    }

    /// Enumerate every currently-registered app. Intended for
    /// debugging; Settings should listen to the relevant events
    /// instead of polling.
    pub async fn list(&self) -> zbus::Result<Vec<(String, String)>> {
        self.proxy.list_apps().await
    }
}

// ---------------------------------------------------------------------------
// org.lunaris.InputManager1 proxy
// ---------------------------------------------------------------------------

#[zbus::proxy(
    interface = "org.lunaris.InputManager1",
    default_service = "org.lunaris.InputManager1",
    default_path = "/org/lunaris/InputManager1"
)]
trait InputManagerBus {
    fn register_binding(
        &self,
        binding: &str,
        action: &str,
        scope: &str,
        app_id: &str,
    ) -> zbus::Result<RegisterResult>;

    fn unregister_binding(&self, binding: &str) -> zbus::Result<bool>;

    fn unregister_all(&self) -> zbus::Result<u32>;

    fn query_bindings(&self, scope_filter: &str) -> zbus::Result<Vec<ClientBindingInfo>>;

    fn query_conflicts(&self, binding: &str) -> zbus::Result<Vec<ConflictInfo>>;

    #[zbus(signal)]
    fn binding_invoked(&self, binding: String, action: String) -> zbus::Result<()>;
}

/// Mirror of the compositor's `BindingInfo`. Kept named distinctly
/// from the compositor-internal struct so downstream crates can tell
/// at a glance whether they are talking to the client-side wire type
/// or the server-side state.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ClientBindingInfo {
    pub binding: String,
    pub action: String,
    pub scope: String,
    pub owner: String,
    pub app_id: String,
}

/// Convenience wrapper around the generated [`InputManagerBusProxy`].
pub struct InputManagerClient<'c> {
    proxy: InputManagerBusProxy<'c>,
}

impl<'c> InputManagerClient<'c> {
    /// Connect to `org.lunaris.InputManager1` on the session bus.
    pub async fn new(conn: &zbus::Connection) -> zbus::Result<Self> {
        let proxy = InputManagerBusProxy::builder(conn)
            .destination(INPUT_SERVICE)?
            .path(INPUT_PATH)?
            .build()
            .await?;
        Ok(Self { proxy })
    }

    /// Request a binding. `scope` is `"app_focused"` or `"app_global"`;
    /// `app_id` is the same identifier the caller passed to
    /// [`AppClient::register`] for `app_focused`, and any string (the
    /// empty one is conventional) for `app_global`.
    pub async fn register_binding(
        &self,
        binding: &str,
        action: &str,
        scope: &str,
        app_id: &str,
    ) -> zbus::Result<RegisterResult> {
        self.proxy
            .register_binding(binding, action, scope, app_id)
            .await
    }

    pub async fn unregister_binding(&self, binding: &str) -> zbus::Result<bool> {
        self.proxy.unregister_binding(binding).await
    }

    pub async fn unregister_all(&self) -> zbus::Result<u32> {
        self.proxy.unregister_all().await
    }

    /// List every binding known to the compositor. `scope_filter` is
    /// `""` for all or one of `"app_focused"` / `"app_global"`.
    pub async fn query_bindings(
        &self,
        scope_filter: &str,
    ) -> zbus::Result<Vec<ClientBindingInfo>> {
        self.proxy.query_bindings(scope_filter).await
    }

    /// Describe every registration that would collide with `binding`.
    pub async fn query_conflicts(&self, binding: &str) -> zbus::Result<Vec<ConflictInfo>> {
        self.proxy.query_conflicts(binding).await
    }

    /// Subscribe to the `BindingInvoked` signal, yielding `(binding,
    /// action)` pairs. Signals are routed by the compositor to the
    /// owning client only, so this stream only sees invocations of
    /// this caller's own bindings.
    pub async fn listen(
        &self,
    ) -> zbus::Result<impl futures_util::Stream<Item = (String, String)> + '_> {
        use futures_util::StreamExt;
        let stream = self.proxy.receive_binding_invoked().await?;
        Ok(stream.filter_map(|signal| async move {
            signal.args().ok().map(|args| (args.binding, args.action))
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_action_serialises_round_trip() {
        let a = DeclaredAction {
            id: "save".into(),
            label: "Save File".into(),
            description: "Persist the current document".into(),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: DeclaredAction = serde_json::from_str(&j).unwrap();
        assert_eq!(back.id, a.id);
        assert_eq!(back.label, a.label);
        assert_eq!(back.description, a.description);
    }

    #[test]
    fn register_result_success_has_empty_conflict_vec() {
        // Asserts the convention all server-side helpers rely on:
        // success=true implies conflict.is_empty(), so clients can
        // use the shorter `if !result.success` check.
        let r = RegisterResult {
            success: true,
            conflict: Vec::new(),
        };
        assert!(r.success && r.conflict.is_empty());
    }

    #[test]
    fn register_result_failure_carries_conflict() {
        let r = RegisterResult {
            success: false,
            conflict: vec![ConflictInfo {
                binding: "Ctrl+S".into(),
                existing_action: "other:save".into(),
                existing_scope: "app_focused".into(),
                existing_owner: ":1.42".into(),
            }],
        };
        assert!(!r.success);
        assert_eq!(r.conflict.len(), 1);
    }
}
