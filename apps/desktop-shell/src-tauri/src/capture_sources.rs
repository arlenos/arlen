// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What there is to share, for the screencast source picker.
//!
//! The picker (`sourcePicker.ts`) is the consent moment when an app asks for
//! the screen: it has to name the monitors and the windows so a person can
//! pick one and deny the rest. That list is the only part of the screencast
//! group that does not wait on a PipeWire producer - the shell already tracks
//! both halves for its own bars and window list, so this is a read of state
//! that is already current rather than a new subscription.
//!
//! The four commands beside it (`start_screencast`, `stop_capture`,
//! `cancel_screencast`, `capture_status`) still need the producer that makes
//! the portal's `Start` return real node ids. Listing what could be shared
//! claims nothing about being able to share it, so it lands alone.

use std::sync::Arc;

use serde::Serialize;

use crate::output_bars::OutputConnectorTable;
use crate::wayland_client::WindowList;

/// A monitor, as the picker shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Monitor {
    /// The connector (`DP-1`), which is what the portal binds a source by.
    pub id: String,
    /// What to put in front of a person: the monitor's own description when
    /// xdg-output gave one, the connector when it did not. Never both, and
    /// never an invented model name.
    pub name: String,
    /// `3840 x 2160`, or empty when the mode is not known yet. An empty
    /// string rather than a guess: a resolution nobody measured, printed
    /// under a monitor's name, is exactly the kind of confident wrong answer
    /// this tree keeps finding.
    pub resolution: String,
}

/// A window, as the picker shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Win {
    pub id: String,
    /// The application id. The picker labels the row with it, so a window
    /// with no app id keeps its title as the only thing said about it.
    #[serde(rename = "appLabel")]
    pub app_label: String,
    pub title: String,
}

/// Both lists, the shape `sourcePicker.ts` parses.
#[derive(Debug, Clone, Serialize)]
pub struct Sources {
    pub monitors: Vec<Monitor>,
    pub windows: Vec<Win>,
}

/// Turn the shell's own two caches into the picker's lists.
///
/// Pure, so the naming rules above are testable without a compositor.
pub fn sources_from(
    outputs: Vec<crate::output_bars::OutputGeometry>,
    windows: &[crate::wayland_client::ToplevelPayload],
) -> Sources {
    let monitors = outputs
        .into_iter()
        .map(|o| Monitor {
            name: o
                .description
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| o.connector.clone()),
            resolution: match o.size {
                Some((w, h)) => format!("{w} x {h}"),
                None => String::new(),
            },
            id: o.connector,
        })
        .collect();
    let windows = windows
        .iter()
        // A window with neither a title nor an app id is a row a person
        // cannot choose between, so it is not offered.
        .filter(|w| !w.title.trim().is_empty() || !w.app_id.trim().is_empty())
        .map(|w| Win {
            id: w.id.clone(),
            app_label: w.app_id.clone(),
            title: w.title.clone(),
        })
        .collect();
    Sources { monitors, windows }
}

/// The monitors and windows an app could be given.
#[tauri::command]
pub fn list_capture_sources(
    outputs: tauri::State<'_, Arc<OutputConnectorTable>>,
    windows: tauri::State<'_, WindowList>,
) -> Sources {
    let held = windows.lock().unwrap_or_else(|e| e.into_inner());
    sources_from(outputs.snapshot(), &held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_bars::OutputGeometry;
    use crate::wayland_client::ToplevelPayload;

    fn output(connector: &str, description: Option<&str>, size: Option<(i32, i32)>) -> OutputGeometry {
        OutputGeometry {
            x: 0,
            y: 0,
            connector: connector.to_string(),
            description: description.map(str::to_string),
            size,
        }
    }

    fn window(id: &str, app_id: &str, title: &str) -> ToplevelPayload {
        ToplevelPayload {
            id: id.to_string(),
            title: title.to_string(),
            app_id: app_id.to_string(),
            active: false,
            minimized: false,
            fullscreen: false,
            workspace_ids: Vec::new(),
            output_connectors: Vec::new(),
        }
    }

    #[test]
    fn a_monitor_is_named_by_its_description_and_falls_back_to_the_connector() {
        let s = sources_from(
            vec![
                output("DP-1", Some("Dell Inc. DELL U2720Q"), Some((3840, 2160))),
                output("HDMI-A-1", None, Some((1920, 1080))),
                output("DP-2", Some("   "), None),
            ],
            &[],
        );
        assert_eq!(s.monitors[0].name, "Dell Inc. DELL U2720Q");
        assert_eq!(s.monitors[0].id, "DP-1");
        assert_eq!(s.monitors[0].resolution, "3840 x 2160");
        assert_eq!(s.monitors[1].name, "HDMI-A-1");
        // Blank is not a name, and an unknown mode is not a resolution.
        assert_eq!(s.monitors[2].name, "DP-2");
        assert_eq!(s.monitors[2].resolution, "");
    }

    #[test]
    fn a_window_with_nothing_to_call_it_is_not_offered() {
        let s = sources_from(
            vec![],
            &[
                window("1", "org.gnome.Calculator", "Calculator"),
                window("2", "", ""),
                window("3", "", "Untitled document"),
            ],
        );
        let ids: Vec<&str> = s.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["1", "3"]);
        assert_eq!(s.windows[0].app_label, "org.gnome.Calculator");
    }
}
