// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Hear the session's accessibility flag and hand it to the grid.
//!
//! A terminal is the surface where "build an accessibility tree" is not a
//! detail: what a person reads is a canvas of glyphs, and to a screen reader a
//! canvas is nothing at all. xterm.js keeps a hidden live-region mirror of the
//! grid and only maintains it when told to, because doing it always costs a DOM
//! write per line on every render. So the flag is the difference between a
//! usable terminal and silence, and it is also the reason it is not just always
//! on.
//!
//! WHY SUBSCRIBE RATHER THAN ASK: the bus retains the last event of every
//! `.state` topic and replays it to a new subscriber, so one subscription
//! answers both questions - what is it now, and what changed - with no poll, no
//! startup race, and no second code path that could disagree with the first. A
//! terminal opened an hour into the session hears the flag as it starts.
//!
//! The value is NOT read from the broker here. The session has one publisher
//! (the shell) so every app hears the same thing at the same time; an app that
//! also read the source directly would be a second answer nobody reconciles.

use tauri::{AppHandle, Emitter};

/// The Tauri event the webview listens on. Payload is the boolean.
pub const TAURI_EVENT: &str = "arlen://accessibility-changed";

/// Subscribe to `accessibility.state` and forward each value to the webview.
///
/// Best-effort by design: a session with no event bus is a terminal that works
/// without the flag, not a terminal that refuses to start. It logs and stops
/// rather than retrying forever, because the bus is present for the life of a
/// session or not at all.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};

        let socket =
            os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock");
        let consumer = UnixEventConsumer::new(socket.to_string_lossy().to_string());
        let mut events = match consumer.subscribe(vec!["accessibility.state".into()]).await {
            Ok(rx) => rx,
            Err(e) => {
                log::info!("terminal: no accessibility feed ({e}); the grid stays as it is");
                return;
            }
        };

        while let Some(event) = events.recv().await {
            use prost::Message;
            match os_sdk::proto::AccessibilityStatePayload::decode(event.payload.as_slice()) {
                Ok(state) => {
                    log::info!("terminal: screen reader {}", state.screen_reader);
                    let _ = app.emit(TAURI_EVENT, state.screen_reader);
                }
                // A payload this app cannot read is the publisher's problem, and
                // guessing a boolean from it would be worse than ignoring it.
                Err(e) => log::warn!("terminal: undecodable accessibility state ({e})"),
            }
        }
    });
}
