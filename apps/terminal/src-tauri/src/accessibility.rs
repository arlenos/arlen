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

use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// How long to wait for the first snapshot before saying nothing arrived.
///
/// The bus replays a retained `.state` topic on subscribe and the shell
/// publishes once at session start, so this is generous rather than tight: it
/// only has to outlast a slow start, not a quiet period.
const FIRST_EVENT_GRACE: Duration = Duration::from_secs(20);

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

        // The bus answers a subscription it will not honour by granting NOTHING,
        // not by refusing: `permitted_subscriptions` filters patterns the caller's
        // `[event_bus].subscribe` scope does not cover, and the connection then
        // succeeds with an empty grant. A consumer cannot tell that apart from a
        // quiet topic, so it waits forever and the grid stays a canvas nobody can
        // read, with nothing anywhere saying why.
        //
        // The terminal is exempt from that filter TODAY only because it declares
        // no event-bus scope, which is the state the profile work (GAP-17) is
        // moving away from. Writing a subscribe list for it and forgetting this
        // topic is a one-line change with a silent, invisible cost.
        //
        // So: say it. The shell publishes once at session start whatever the value
        // is, so hearing nothing at all means something is wrong rather than the
        // flag being off - but this deliberately does not GUESS which thing (a
        // filtered pattern, a bus that never came up, a broker the shell could not
        // read all look the same from here). It reports what it knows.
        let first = tokio::time::timeout(FIRST_EVENT_GRACE, events.recv()).await;
        let mut pending = match first {
            Ok(Some(event)) => Some(event),
            // The producer side hung up.
            Ok(None) => return,
            Err(_) => {
                log::warn!(
                    "terminal: nothing on accessibility.state after {}s - the grid keeps its \
                     current screen-reader setting. The subscription may have been filtered \
                     (no `[event_bus].subscribe` grant for it), or nothing is publishing.",
                    FIRST_EVENT_GRACE.as_secs()
                );
                None
            }
        };

        loop {
            let event = match pending.take() {
                Some(e) => e,
                None => match events.recv().await {
                    Some(e) => e,
                    None => return,
                },
            };
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
