//! `org.freedesktop.impl.portal.ScreenCast` implementation (capture-active #12).
//!
//! The impl-side backend behind the `xdg-desktop-portal` frontend for screen
//! sharing. The freedesktop flow is stateful across three calls on one session:
//! `CreateSession` opens the session, `SelectSources` is where the backend shows
//! the source picker the app cannot see and the user consents, and `Start`
//! begins the PipeWire stream (returning the node ids + an optional
//! `restore_token`). Implemented IN the Arlen backend over PipeWire, never
//! delegated to `xdg-desktop-portal-wlr`.
//!
//! Spec:
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.ScreenCast.html
//!
//! Build status: the session model, the frontend-only gate, the no-silent-capture
//! audit and the CONSENT are real. `SelectSources` routes through the consent
//! broker as `ConsentClass::ScreenCast` on behalf of the capturing app, and
//! fails closed - a denial or an unreachable broker refuses the share.
//!
//! What remains is the PipeWire producer that makes `Start` return real node
//! ids, which is why this interface is served on the bus but deliberately not
//! listed in `arlen.portal`'s `Interfaces` line: advertising a `Start` that
//! cannot stream would offer the frontend a capability that does not work.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use audit_proto::sink::{AuditSink, LedgerAuditSink};
use audit_proto::{AuditKind, IngestRequest, StructuralRecord};
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::interfaces::sender_is_frontend;
use crate::request::{response, RequestHandle};
use crate::state::DaemonState;

/// `AvailableSourceTypes` bitmask (portal spec): monitor + window. Virtual
/// (`4`) is a later phase once the PipeWire producer can synthesise a source.
const SOURCE_TYPES: u32 = 1 /* monitor */ | 2 /* window */;

/// `AvailableCursorModes` bitmask: hidden, embedded, metadata (all three are
/// producer options; the producer honours the selected one).
const CURSOR_MODES: u32 = 1 /* hidden */ | 2 /* embedded */ | 4 /* metadata */;

/// The interface version implemented. v4 introduced the `restore_token`
/// persistence (decision 3 honours it); v5 is the current wire version.
const VERSION: u32 = 5;

/// One in-progress screencast session, keyed by its session handle. Records the
/// app's `SelectSources` choices so `Start` streams exactly what was consented.
#[derive(Debug, Clone)]
struct ScreenCastSession {
    /// The requesting app id (frontend-attested).
    app_id: String,
    /// The selected source-type bitmask (monitor/window/virtual).
    source_types: u32,
    /// The selected cursor mode.
    cursor_mode: u32,
    /// The persist mode (0 none / 1 transient / 2 persistent - decision 3).
    persist_mode: u32,
}

/// Build the content-free audit event for one screencast lifecycle step
/// (capture-active-infra-plan.md: the no-silent-capture principle - "app X
/// shared the screen at time T" is recorded, every time, never the pixels). The
/// STRUCTURAL tier stays content-free: only the coarse app id + the lifecycle
/// outcome, never the source geometry, the window title or a frame.
/// [`AuditKind::Permission`]: a mediated, permitted, privacy-sensitive act.
fn screencast_audit_event(app_id: &str, outcome: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: "capture.screencast".to_string(),
            node_types: vec![app_id.to_string()],
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// The `org.freedesktop.impl.portal.ScreenCast` backend.
#[derive(Clone)]
pub struct ScreenCast {
    state: DaemonState,
    /// The audit ledger sink: every share step is recorded (no silent capture).
    audit: Arc<dyn AuditSink>,
    /// In-progress sessions, shared across served clones.
    sessions: Arc<Mutex<HashMap<String, ScreenCastSession>>>,
}

impl ScreenCast {
    /// Build the interface over the shared daemon state, recording to the
    /// default audit ledger socket.
    pub fn new(state: DaemonState) -> Self {
        Self::with_audit(state, Arc::new(LedgerAuditSink::at_default_socket()))
    }

    /// Build over an injected audit sink (tests supply a mock).
    pub fn with_audit(state: DaemonState, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            state,
            audit,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// A `{"arlen-error": message}` results map for a failed call.
fn error_results(message: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(owned) = Value::new(message.to_string()).try_to_owned() {
        map.insert("arlen-error".to_string(), owned);
    }
    map
}

/// Whether `caller` owns the session keyed by `key` (session isolation between
/// apps: only the app that created a session may configure or start it). A
/// missing session is not owned.
fn session_owned_by(
    sessions: &HashMap<String, ScreenCastSession>,
    key: &str,
    caller: &str,
) -> Result<(), &'static str> {
    match sessions.get(key) {
        Some(s) if s.app_id == caller => Ok(()),
        Some(_) => Err("session belongs to a different app"),
        None => Err("unknown screencast session"),
    }
}

/// Read a `u32` option value, defaulting when absent or the wrong type.
fn opt_u32(options: &HashMap<&str, OwnedValue>, key: &str, default: u32) -> u32 {
    options
        .get(key)
        .and_then(|v| u32::try_from(v.clone()).ok())
        .unwrap_or(default)
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    /// The implemented interface version.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        VERSION
    }

    /// The source types this backend can offer.
    #[zbus(property, name = "AvailableSourceTypes")]
    fn available_source_types(&self) -> u32 {
        SOURCE_TYPES
    }

    /// The cursor modes this backend supports.
    #[zbus(property, name = "AvailableCursorModes")]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_MODES
    }

    /// Open a screencast session. Tracks it until `Start` or the frontend
    /// closes it.
    async fn create_session(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            tracing::warn!("refusing a ScreenCast.CreateSession from a non-frontend sender");
            return (
                response::OTHER,
                error_results("caller is not the xdg-desktop-portal frontend"),
            );
        }
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        let key = session_handle.as_str().to_string();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(
                key.clone(),
                ScreenCastSession {
                    app_id: app_id.to_string(),
                    source_types: 0,
                    cursor_mode: 0,
                    persist_mode: 0,
                },
            );
        }
        tracing::info!(request = %req.path, session = %key, app_id, "ScreenCast: session created");
        (response::SUCCESS, HashMap::new())
    }

    /// The app selects which sources to share; the backend shows the picker the
    /// app cannot see and the user consents. Records the selection on the
    /// session.
    ///
    /// The broker-consent mint (`ConsentClass::ScreenCast` → a revocable LCG
    /// grant) is the next slice; the choices are captured here so that mint has
    /// its scope and `Start` streams exactly what was consented.
    async fn select_sources(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            tracing::warn!("refusing a ScreenCast.SelectSources from a non-frontend sender");
            return (
                response::OTHER,
                error_results("caller is not the xdg-desktop-portal frontend"),
            );
        }
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        let key = session_handle.as_str().to_string();

        let types = opt_u32(&options, "types", SOURCE_TYPES) & SOURCE_TYPES;
        let cursor_mode = opt_u32(&options, "cursor_mode", 2) & CURSOR_MODES;
        let persist_mode = opt_u32(&options, "persist_mode", 0);

        // Session isolation: only the app that created the session may
        // configure it (the stored app_id is frontend-attested at CreateSession).
        let outcome = if let Ok(mut sessions) = self.sessions.lock() {
            match sessions.get_mut(&key) {
                Some(session) if session.app_id == app_id => {
                    session.source_types = types;
                    session.cursor_mode = cursor_mode;
                    session.persist_mode = persist_mode;
                    Ok(())
                }
                Some(_) => Err("session belongs to a different app"),
                None => Err("unknown screencast session"),
            }
        } else {
            Err("session store unavailable")
        };
        if let Err(msg) = outcome {
            tracing::warn!(request = %req.path, session = %key, msg, "SelectSources refused");
            return (response::OTHER, error_results(msg));
        }

        // Route consent through the broker ON BEHALF OF the capturing app: the
        // grant + the dialog name app_id (the frontend-verified id), not the
        // portal (the broker honors on_behalf_of only for the allowlisted
        // portal). Fail-closed - a denial or an unreachable broker refuses the
        // share. Blocks until the user resolves the trusted-path dialog.
        let decision = crate::consent::request_screencast_consent(
            &crate::consent::intake_socket_path(),
            app_id,
            "Share your screen contents",
        )
        .await;
        match decision {
            crate::consent::ConsentDecision::Allowed => {
                tracing::info!(request = %req.path, session = %key, app_id, types, cursor_mode, persist_mode, "ScreenCast: sources selected, consent granted");
                (response::SUCCESS, HashMap::new())
            }
            crate::consent::ConsentDecision::Denied => {
                tracing::info!(request = %req.path, session = %key, app_id, "ScreenCast: consent denied");
                (response::CANCELLED, HashMap::new())
            }
        }
    }

    /// Start the stream. The PipeWire producer that returns real node ids is the
    /// next slice; until then this fails cleanly (never a fake empty stream) and
    /// records the attempt.
    // The argument list is the fixed `org.freedesktop.impl.portal.ScreenCast`
    // Start D-Bus signature, not a design smell.
    #[allow(clippy::too_many_arguments)]
    async fn start(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            tracing::warn!("refusing a ScreenCast.Start from a non-frontend sender");
            return (
                response::OTHER,
                error_results("caller is not the xdg-desktop-portal frontend"),
            );
        }
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        let key = session_handle.as_str().to_string();
        // Session isolation: only the creating app may start its own session.
        let ownership = self
            .sessions
            .lock()
            .map(|s| session_owned_by(&s, &key, app_id))
            .unwrap_or(Err("session store unavailable"));
        if let Err(msg) = ownership {
            return (response::OTHER, error_results(msg));
        }

        // No-silent-capture: record the share attempt even while streaming is
        // pending, best-effort like the sibling capture backends.
        if let Err(e) = self
            .audit
            .submit(screencast_audit_event(app_id, "stream-pending"))
            .await
        {
            tracing::warn!(request = %req.path, "ScreenCast audit record failed: {e}");
        }
        tracing::info!(request = %req.path, session = %key, app_id, "ScreenCast: Start (PipeWire producer pending)");
        (
            response::OTHER,
            error_results("screencast streaming is not yet available (PipeWire producer pending)"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_capabilities_match_the_spec_bitmasks() {
        // Monitor + window offered; virtual deferred to the producer phase.
        assert_eq!(SOURCE_TYPES, 0b011);
        // Hidden + embedded + metadata cursor modes.
        assert_eq!(CURSOR_MODES, 0b111);
        assert_eq!(VERSION, 5);
    }

    #[test]
    fn a_share_step_records_the_app_and_outcome_content_free() {
        let req = screencast_audit_event("org.example.sharer", "stream-pending");
        assert_eq!(req.kind, AuditKind::Permission, "a share is a permitted, mediated act");
        assert_eq!(req.structural.subject, "capture.screencast");
        assert_eq!(req.structural.node_types, vec!["org.example.sharer"]);
        assert_eq!(req.structural.outcome, "stream-pending");
        req.validate().expect("within the structural caps");
    }

    #[test]
    fn session_isolation_only_the_creator_may_start() {
        let mut sessions = HashMap::new();
        sessions.insert(
            "/s/1".to_string(),
            ScreenCastSession {
                app_id: "org.example.owner".to_string(),
                source_types: 0,
                cursor_mode: 0,
                persist_mode: 0,
            },
        );
        assert!(session_owned_by(&sessions, "/s/1", "org.example.owner").is_ok());
        assert_eq!(
            session_owned_by(&sessions, "/s/1", "org.example.intruder"),
            Err("session belongs to a different app")
        );
        assert_eq!(
            session_owned_by(&sessions, "/s/absent", "org.example.owner"),
            Err("unknown screencast session")
        );
    }

    #[test]
    fn the_source_geometry_never_reaches_the_structural_tier() {
        let req = screencast_audit_event("app", "stream-pending");
        let haystack = format!(
            "{} {}",
            req.structural.subject,
            req.structural.node_types.join(",")
        );
        // No monitor connector, window title, geometry or frame in the record.
        assert!(!haystack.contains("HDMI"), "no output connector");
        assert!(!haystack.contains("x"), "no WxH geometry token");
        assert!(req.forensic.is_none(), "a share never reaches the forensic tier");
    }
}
