// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! The privacy sentinel's state and switches, read from the daemon that measures
//! them (`privacy-sentinel-plan.md`).
//!
//! WHY THESE FIVE EXIST AT ALL. `sentinel.ts` has called `sentinel_get_state`
//! since 26 August and got nothing back, and the page above it is a page that
//! tells somebody whether their machine is broadcasting a trackable identity.
//! `fixPosture` used to write "Bluetooth is no longer discoverable." into the
//! readout whatever happened, because the command behind it never existed, so the
//! page reported a fix on every machine it never performed one on.
//!
//! Nothing here decides anything. Each command is a round trip to the daemon and
//! back: the daemon holds the switches, does the reading, applies the two
//! remediations and refuses the rest. A refusal comes back as its own sentence
//! rather than an error, because "that did not work" on a privacy page leaves
//! somebody with no idea whether they are protected.
//!
//! `ask` is synchronous one-shot-per-connection, so each call runs on a blocking
//! thread to keep the async runtime free - the same shape as the bottle commands.

use arlen_sentineld::client::ask;
use arlen_sentineld::protocol::{Request, Response, State};
use arlen_sentineld::server::socket_path;

/// Run one ask on a blocking thread and map the answer.
///
/// A `Refused` and a `Failed` both come back as `Err(message)`: the store already
/// renders a failed change by putting the switch back and saying so, and the
/// daemon's message is the sentence worth showing either way.
async fn round_trip(request: Request) -> Result<Response, String> {
    tokio::task::spawn_blocking(move || {
        ask(&socket_path(), &request).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("the privacy sentinel could not be asked: {e}"))?
}

/// Turn a plain answer into a result the surface can act on.
fn done(response: Response) -> Result<(), String> {
    match response {
        Response::Done => Ok(()),
        Response::Refused { message } | Response::Failed { message } => Err(message),
        Response::State(_) => Err("the privacy sentinel answered something else".to_string()),
    }
}

/// Everything the privacy page renders: the detector switches, and the exposure
/// readout when that detector is running.
///
/// An error is the daemon not being reachable, which the store renders as nothing
/// reporting rather than as a machine with nothing wrong. Those are different
/// things and this page is the last one that should conflate them.
#[tauri::command]
pub async fn sentinel_get_state() -> Result<State, String> {
    match round_trip(Request::GetState).await? {
        Response::State(state) => Ok(*state),
        Response::Refused { message } | Response::Failed { message } => Err(message),
        Response::Done => Err("the privacy sentinel answered something else".to_string()),
    }
}

/// Turn a detector on or off.
#[tauri::command]
pub async fn sentinel_set_detector(id: String, on: bool) -> Result<(), String> {
    done(round_trip(Request::SetDetector { id, on }).await?)
}

/// Switch a detector between staying quiet and notifying.
#[tauri::command]
pub async fn sentinel_set_alerts(id: String, mode: String) -> Result<(), String> {
    done(round_trip(Request::SetAlerts { id, mode }).await?)
}

/// Set a watcher's sensitivity.
#[tauri::command]
pub async fn sentinel_set_sensitivity(id: String, level: String) -> Result<(), String> {
    done(round_trip(Request::SetSensitivity { id, level }).await?)
}

/// Apply the one-click remediation behind a posture line.
///
/// Takes the SURFACE, not the line's position. The readout is recomputed on every
/// read and sorted worst-first, so a radio that changed between the read and the
/// tap moves the lines; an index would then point the fix at the neighbouring
/// one. A surface fixes the thing the person tapped or nothing at all.
#[tauri::command]
pub async fn sentinel_fix_posture(surface: String) -> Result<(), String> {
    done(round_trip(Request::FixPosture { surface }).await?)
}
