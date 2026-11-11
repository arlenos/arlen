// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Which screen captures are running, as the compositor announces them.
//!
//! The compositor sees every capture session whichever way it was opened (the
//! portal, a client that captures directly, the shell's own screenshot) and
//! sends `capture_started` and `capture_stopped` on the overlay protocol from
//! version 3. This module holds the sessions between those two events and
//! answers the top-bar badge's `capture_status` read. A session that ends
//! inside [`SHOWN_AT_LEAST_MS`] stays shown until that floor has run out.
//!
//! The portal and PipeWire path of `capture-active-infra-plan.md` stays the
//! source for microphone and camera. This one covers the screen, and it needs
//! no producer: the compositor is the one place every screen capture passes.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::Emitter;

use crate::app_index::{AppEntry, AppIndex};

/// Emitted whenever a capture starts or stops, so the badge reads at once
/// instead of on its next poll. A screenshot starts and stops within moments
/// and a poll alone would never see it.
pub const CHANGED_EVENT: &str = "arlen://capture-changed";

/// How long a capture stays shown at the least, counted from its start. A
/// screenshot is a session of a few milliseconds and the events carry no
/// duration of their own, so without a floor the one capture a person is most
/// likely to miss would never light the badge.
pub const SHOWN_AT_LEAST_MS: u64 = 2_000;

/// One capture session, running or ended inside its floor.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Capture {
    /// The capturing client as the compositor named it.
    app_id: String,
    /// Milliseconds since the epoch at which the shell heard of it.
    started_at_ms: u64,
    /// Set once the compositor reported it gone.
    ended: bool,
}

impl Capture {
    /// Whether the badge still shows this session at `now_ms`.
    fn shown(&self, now_ms: u64) -> bool {
        !self.ended || now_ms < self.started_at_ms.saturating_add(SHOWN_AT_LEAST_MS)
    }
}

/// The capture sessions the badge shows, keyed by the compositor's capture id.
#[derive(Debug, Default)]
pub struct CaptureState {
    running: Mutex<BTreeMap<u32, Capture>>,
}

/// What the badge reads. The field names are the ones `CaptureBadge.svelte`
/// was built against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    /// Whether any capture session is running.
    pub capture_active: bool,
    /// The name of the app that has captured longest, absent when none runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capturing_app_label: Option<String>,
    /// When that capture started, in milliseconds since the epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
}

impl CaptureState {
    /// An empty state: nothing is capturing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a session the compositor announced. A second start for an id
    /// that is still running keeps the first start time: the light has been
    /// on since then.
    pub fn started(&self, capture_id: u32, app_id: String, now_ms: u64) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|_, c| c.shown(now_ms));
        running.entry(capture_id).or_insert(Capture {
            app_id,
            started_at_ms: now_ms,
            ended: false,
        });
    }

    /// Mark a session the compositor reported gone and return how many
    /// milliseconds it stays shown from `now_ms` (0 when it is gone at once).
    /// An unknown id is not an error: the shell may have bound after the
    /// session started.
    pub fn stopped(&self, capture_id: u32, now_ms: u64) -> u64 {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        let left = match running.get_mut(&capture_id) {
            Some(c) => {
                c.ended = true;
                c.started_at_ms
                    .saturating_add(SHOWN_AT_LEAST_MS)
                    .saturating_sub(now_ms)
            }
            None => 0,
        };
        running.retain(|_, c| c.shown(now_ms));
        left
    }

    /// What the badge shows: the session that has run longest names the app
    /// and the clock, because that is the one a person is likeliest to have
    /// forgotten.
    pub fn status(&self, apps: &[AppEntry], now_ms: u64) -> CaptureStatus {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        running.retain(|_, c| c.shown(now_ms));
        let oldest = running.values().min_by_key(|c| c.started_at_ms);
        CaptureStatus {
            capture_active: oldest.is_some(),
            capturing_app_label: oldest.map(|c| label_for(apps, &c.app_id)),
            started_at: oldest.map(|c| c.started_at_ms),
        }
    }
}

/// The name a person knows the capturing client by: the installed app's name
/// when its id matches, else the id as the compositor gave it. Never empty, so
/// the badge's sentence always names something.
fn label_for(apps: &[AppEntry], app_id: &str) -> String {
    name_among(apps.iter().map(|a| (a.app_id.as_str(), a.name.as_str())), app_id)
}

/// [`label_for`] over bare `(app id, name)` pairs.
fn name_among<'a>(mut apps: impl Iterator<Item = (&'a str, &'a str)>, app_id: &str) -> String {
    apps.find(|(id, _)| *id == app_id)
        .map(|(_, name)| name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(app_id)
        .to_string()
}

/// Milliseconds since the epoch, for stamping a start.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Apply a `capture_started` from the overlay protocol and tell the badge.
pub fn on_started(app: &tauri::AppHandle, capture_id: u32, app_id: String) {
    use tauri::Manager;
    if let Some(state) = app.try_state::<std::sync::Arc<CaptureState>>() {
        state.started(capture_id, app_id, now_ms());
    }
    let _ = app.emit(CHANGED_EVENT, ());
}

/// Apply a `capture_stopped` from the overlay protocol and tell the badge,
/// once now and once more when a short session's floor has run out.
pub fn on_stopped(app: &tauri::AppHandle, capture_id: u32) {
    use tauri::Manager;
    let left = app
        .try_state::<std::sync::Arc<CaptureState>>()
        .map(|state| state.stopped(capture_id, now_ms()))
        .unwrap_or(0);
    let _ = app.emit(CHANGED_EVENT, ());
    if left > 0 {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(left + 50));
            let _ = app.emit(CHANGED_EVENT, ());
        });
    }
}

/// Whether the screen is being captured, by whom and since when.
#[tauri::command]
pub fn capture_status(
    state: tauri::State<'_, std::sync::Arc<CaptureState>>,
    index: tauri::State<'_, AppIndex>,
) -> CaptureStatus {
    let apps = index.lock().unwrap_or_else(|e| e.into_inner());
    state.status(&apps, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_running_reads_as_inactive_with_no_label() {
        let state = CaptureState::new();
        let status = state.status(&[], 0);
        assert!(!status.capture_active);
        assert_eq!(status.capturing_app_label, None);
        assert_eq!(status.started_at, None);
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json, serde_json::json!({ "captureActive": false }));
    }

    #[test]
    fn the_longest_running_capture_names_the_badge() {
        let state = CaptureState::new();
        state.started(7, "org.example.Recorder".into(), 1_000);
        state.started(9, "org.example.Meet".into(), 5_000);
        let status = state.status(&[], 60_000);
        assert!(status.capture_active);
        assert_eq!(status.capturing_app_label.as_deref(), Some("org.example.Recorder"));
        assert_eq!(status.started_at, Some(1_000));

        // The older one ends: the light stays on for the one still running.
        assert_eq!(state.stopped(7, 60_000), 0, "long past its floor");
        let status = state.status(&[], 60_000);
        assert!(status.capture_active);
        assert_eq!(status.capturing_app_label.as_deref(), Some("org.example.Meet"));
        assert_eq!(status.started_at, Some(5_000));

        state.stopped(9, 60_000);
        assert!(!state.status(&[], 60_000).capture_active);
    }

    /// A screenshot starts and stops within milliseconds. It stays shown until
    /// the floor has run out, and not a moment of it is lost to a slow read.
    #[test]
    fn a_capture_that_ends_at_once_is_still_shown_for_its_floor() {
        let state = CaptureState::new();
        state.started(1, "grim".into(), 10_000);
        let left = state.stopped(1, 10_004);
        assert_eq!(left, SHOWN_AT_LEAST_MS - 4);
        let status = state.status(&[], 10_500);
        assert!(status.capture_active, "inside the floor");
        assert_eq!(status.capturing_app_label.as_deref(), Some("grim"));
        assert!(!state.status(&[], 10_000 + SHOWN_AT_LEAST_MS).capture_active, "floor over");
    }

    #[test]
    fn a_repeated_start_keeps_the_first_time_and_an_unknown_stop_is_ignored() {
        let state = CaptureState::new();
        state.started(3, "a".into(), 100);
        state.started(3, "a".into(), 900);
        assert_eq!(state.status(&[], 900).started_at, Some(100));
        assert_eq!(state.stopped(42, 900), 0);
        assert!(state.status(&[], 99_000).capture_active);
    }

    #[test]
    fn an_installed_app_is_named_by_its_name() {
        let apps = [("dev.arlen.screenshot", "Screenshot"), ("dev.arlen.blank", " ")];
        assert_eq!(name_among(apps.into_iter(), "dev.arlen.screenshot"), "Screenshot");
        assert_eq!(name_among(apps.into_iter(), "grim"), "grim");
        assert_eq!(name_among(apps.into_iter(), "dev.arlen.blank"), "dev.arlen.blank");
    }

    #[test]
    fn the_wire_names_are_the_ones_the_badge_reads() {
        let state = CaptureState::new();
        state.started(1, "x".into(), 12);
        let json = serde_json::to_value(state.status(&[], 12)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "captureActive": true,
                "capturingAppLabel": "x",
                "startedAt": 12
            })
        );
    }
}
