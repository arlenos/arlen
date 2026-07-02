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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::request::{response, RequestHandle};
use crate::state::DaemonState;

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
}

impl Screenshot {
    /// Build the interface over the shared daemon state.
    pub fn new(state: DaemonState) -> Self {
        Self { state }
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
