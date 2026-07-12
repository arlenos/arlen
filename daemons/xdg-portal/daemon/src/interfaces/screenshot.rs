//! `org.freedesktop.impl.portal.Screenshot` implementation.
//!
//! The impl-side backend behind the `xdg-desktop-portal` frontend. On
//! `Screenshot` it captures via the Arlen capture core
//! (`ext-image-copy-capture`, never the deprecated wlr-screencopy nor a
//! nested portal), saves the PNG to the screenshots directory, and returns
//! its `file://` URI in `results`. The frontend owns the app-facing Request
//! object and the Document Portal export; this backend does the capture.
//!
//! Spec:
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Screenshot.html
//!
//! `interactive` (region/window picker) is not yet honoured: the request is
//! served as a full-primary-output capture with a log line. The interactive
//! picker reuses the picker-ui pattern and lands as a follow-up. `PickColor`
//! returns OTHER "not implemented" so callers fall through, exactly as the
//! OpenURI `OpenFile` fd path does.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audit_proto::sink::{AuditSink, LedgerAuditSink};
use audit_proto::{AuditKind, IngestRequest, StructuralRecord};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::request::{response, RequestHandle};
use crate::state::DaemonState;

/// Build the content-free audit event for one screen capture (screen-capture-plan.md
/// §6.2, the no-silent-capture principle: "app X captured the screen at time T" is
/// recorded, every time). The STRUCTURAL tier stays content-free - the calling app
/// id is a coarse label (the audit daemon attributes the connection actor to the
/// portal), and the captured IMAGE and its path are never recorded. Kind
/// [`AuditKind::Permission`]: a capture is a mediated, permitted, privacy-sensitive
/// action, not an egress.
fn screenshot_audit_event(app_id: &str, outcome: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: "capture.screenshot".to_string(),
            // Who captured - a coarse id. No image, no output geometry, no path.
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

/// Percent-encoding set for a `file://` URI path: controls plus the
/// characters that are unsafe or delimiters in a URI path segment.
const URI_PATH_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// The `org.freedesktop.impl.portal.Screenshot` backend.
#[derive(Clone)]
pub struct Screenshot {
    state: DaemonState,
    /// The audit ledger sink: every capture is recorded (no silent capture).
    audit: Arc<dyn AuditSink>,
}

impl Screenshot {
    /// Build the interface over the shared daemon state, recording captures to the
    /// default audit ledger socket.
    pub fn new(state: DaemonState) -> Self {
        Self::with_audit(state, Arc::new(LedgerAuditSink::at_default_socket()))
    }

    /// Build over an injected audit sink (tests supply a mock).
    pub fn with_audit(state: DaemonState, audit: Arc<dyn AuditSink>) -> Self {
        Self { state, audit }
    }
}

fn error_results(message: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(owned) = Value::new(message.to_string()).try_to_owned() {
        map.insert("arlen-error".to_string(), owned);
    }
    map
}

/// A unique screenshot path in the screenshots directory. Uses an epoch
/// millisecond suffix for uniqueness (the frontend exposes the file to the
/// requesting app via the Document Portal; the on-disk name is internal).
fn screenshot_path() -> std::path::PathBuf {
    let dir = arlen_screen_capture::screenshots_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    dir.join(format!("Screenshot-{stamp}.png"))
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    /// Interface version. v2 adds `PickColor`.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    /// Capture a screenshot and return its `file://` URI.
    async fn screenshot(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        options: HashMap<&str, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        let interactive = options
            .get("interactive")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);

        if interactive {
            // Not yet honoured: a region/window picker is a follow-up. Serve a
            // full-output capture so the request still yields a screenshot.
            tracing::info!(
                request = %req.path,
                app_id,
                parent_window,
                "Screenshot: interactive requested, serving full-output capture (picker is a follow-up)"
            );
        } else {
            tracing::info!(
                request = %req.path,
                app_id,
                parent_window,
                "Screenshot: full-output capture"
            );
        }

        // The capture core is a blocking Wayland client; keep it off the reactor and
        // bound it so a misbehaving compositor can never hang a D-Bus request forever.
        const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
        let capture = tokio::time::timeout(
            CAPTURE_TIMEOUT,
            tokio::task::spawn_blocking(|| arlen_screen_capture::capture_output(0, false)),
        )
        .await;
        // The no-silent-capture principle (SC-R6): record every capture attempt +
        // outcome, content-free. A SUCCESSFUL capture that cannot be recorded is
        // refused - the image is never handed back unaudited (fail-closed for a
        // privacy-sensitive act). A FAILED capture read nothing, so its record is
        // best-effort (a warn, not a refusal).
        let succeeded = matches!(capture, Ok(Ok(Ok(_))));
        let outcome = match &capture {
            Ok(Ok(Ok(_))) => "captured",
            Ok(Ok(Err(_))) => "capture-failed",
            Ok(Err(_)) => "capture-panicked",
            Err(_) => "capture-timed-out",
        };
        let audit_result = self.audit.submit(screenshot_audit_event(app_id, outcome)).await;
        if succeeded {
            if let Err(e) = audit_result {
                tracing::warn!(request = %req.path, "Screenshot audit failed; refusing to return the capture: {e}");
                return (response::OTHER, error_results("capture could not be recorded"));
            }
        } else if let Err(e) = audit_result {
            tracing::warn!(request = %req.path, "Screenshot audit of a failed capture could not be recorded: {e}");
        }

        let image = match capture {
            Ok(Ok(Ok(image))) => image,
            Ok(Ok(Err(e))) => {
                tracing::warn!(request = %req.path, "Screenshot capture failed: {e}");
                return (response::OTHER, error_results(&format!("capture failed: {e}")));
            }
            Ok(Err(e)) => {
                tracing::warn!(request = %req.path, "Screenshot capture task panicked: {e}");
                return (response::OTHER, error_results("capture task failed"));
            }
            Err(_elapsed) => {
                tracing::warn!(request = %req.path, "Screenshot capture timed out");
                return (response::OTHER, error_results("capture timed out"));
            }
        };

        let path = screenshot_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return (
                    response::OTHER,
                    error_results(&format!("create screenshots dir: {e}")),
                );
            }
        }
        if let Err(e) = arlen_screen_capture::write_png(&image, &path) {
            tracing::warn!(request = %req.path, "Screenshot write failed: {e}");
            return (response::OTHER, error_results(&format!("write PNG: {e}")));
        }

        let uri = format!(
            "file://{}",
            utf8_percent_encode(&path.to_string_lossy(), URI_PATH_SET)
        );
        tracing::info!(request = %req.path, path = %path.display(), "Screenshot saved");

        let mut results = HashMap::new();
        match Value::new(uri).try_to_owned() {
            Ok(owned) => {
                results.insert("uri".to_string(), owned);
                (response::SUCCESS, results)
            }
            Err(e) => (response::OTHER, error_results(&format!("encode uri: {e}"))),
        }
    }

    /// Pick a color from the screen. Not yet implemented (needs an
    /// interactive pixel picker); returns OTHER so callers fall through.
    async fn pick_color(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<&str, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        tracing::info!(request = %req.path, app_id, "PickColor requested (not implemented)");
        (
            response::OTHER,
            error_results("PickColor is not yet implemented"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_records_the_app_and_outcome_content_free() {
        let req = screenshot_audit_event("org.example.recorder", "captured");
        assert_eq!(req.kind, AuditKind::Permission, "a capture is a permitted, mediated act");
        assert_eq!(req.structural.subject, "capture.screenshot");
        assert_eq!(req.structural.node_types, vec!["org.example.recorder"]);
        assert_eq!(req.structural.outcome, "captured");
        req.validate().expect("within the structural caps");
    }

    #[test]
    fn the_image_and_path_never_reach_the_structural_tier() {
        let req = screenshot_audit_event("app", "captured");
        // Only the coarse app id is carried; no path, no output geometry, no bytes.
        let haystack = format!("{} {}", req.structural.subject, req.structural.node_types.join(","));
        assert!(!haystack.contains("/"), "no filesystem path in the record");
        assert!(!haystack.contains(".png"), "no image file name in the record");
        assert!(req.forensic.is_none(), "the capture never reaches the forensic tier");
    }

    #[test]
    fn a_failed_capture_records_its_outcome() {
        for outcome in ["capture-failed", "capture-panicked", "capture-timed-out"] {
            let req = screenshot_audit_event("app", outcome);
            assert_eq!(req.structural.outcome, outcome);
            req.validate().expect("within the structural caps");
        }
    }
}
