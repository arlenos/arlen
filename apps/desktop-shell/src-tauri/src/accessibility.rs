// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Put the session's accessibility settings on the event bus.
//!
//! The flag lives in the config broker, which runs as its own uid so a same-uid
//! process cannot rewrite it. That uid is exactly why the broker cannot publish
//! this itself: the event bus producer socket is under `/run/user/<uid>/arlen`,
//! the session user's own runtime directory, and the broker is not that user.
//! So the shell reads the value it does not own and republishes it for the
//! session, the same way it already republishes audio.
//!
//! One flag today - whether a screen reader is in use - and it is not a
//! preference in the sense a theme is. Getting it wrong costs somebody the use
//! of the machine, so it is deliberately NOT part of appearance and cannot be
//! changed as a side effect of changing how things look.
//!
//! A poll rather than a subscription, because the broker's protocol has no
//! watch op. Two seconds is a boolean nobody toggles twice in a row, and a
//! publish only happens when the value actually changed, so the bus sees one
//! event per change rather than one every tick.
//!
//! LATE JOINERS ARE FREE: the bus retains the last event of any `.state` topic
//! and delivers it on subscribe, so an app that starts an hour into the session
//! gets the flag immediately rather than waiting for somebody to toggle it. That
//! is why the topic is named `accessibility.state` and not `accessibility.changed`
//! - the retention is derived from the name.

use std::time::Duration;

/// How often the broker is re-read. Only a CHANGE publishes.
const POLL: Duration = Duration::from_secs(2);

/// The variable the greeter hands the login screen's choice on in.
///
/// Kept as a literal rather than a dep on the greeter crate: the shell has no
/// other reason to link the login screen, and the string is the contract. It
/// matches `arlen_greeter_core::A11Y_SCREEN_READER_ENV`.
const HANDOFF_ENV: &str = "ARLEN_A11Y_SCREEN_READER";

/// Record the login screen's choice in the user's own config, once, at session
/// start.
///
/// ABSENT MEANS "NOTHING TO SAY", NOT "OFF". The greeter only sets the variable
/// when the toggle is on, so a login screen nobody touched leaves whatever the
/// person already had. Turning a screen reader off is a thing somebody does
/// deliberately in Settings, never a side effect of logging in.
///
/// The write is what makes this stick: the greeter cannot read the broker back
/// (different uid, and before login no user is chosen), but the SHELL reads it
/// every session. So somebody turns it on at the login screen once, the session
/// records it, and every later session already has it without the login screen
/// being involved at all.
async fn record_login_choice() {
    if std::env::var(HANDOFF_ENV).ok().as_deref() != Some("1") {
        return;
    }
    let client = arlen_config_broker::ConfigBrokerClient::default_socket();
    // Read first: if it is already on, there is nothing to record, and a write
    // that changes nothing is a write that can still fail.
    match client.get().await {
        Ok(state) if state.accessibility.screen_reader => return,
        Ok(_) => {}
        Err(e) => {
            log::warn!("accessibility: cannot record the login choice, broker unreadable ({e})");
            return;
        }
    }
    let wanted = arlen_config_broker::Accessibility { screen_reader: true };
    match client.set_accessibility(wanted).await {
        Ok(()) => log::info!("accessibility: recorded the login screen's screen-reader choice"),
        // Worth a warning rather than silence: the person will find it off again
        // next login and have no idea why.
        Err(e) => log::warn!("accessibility: could not record the login choice ({e})"),
    }
}

/// Read the current flag from the broker.
///
/// `None` means the broker did not answer or could not be trusted (down,
/// corrupt, refused). That is deliberately not "false": publishing false for an
/// unreachable broker would turn a transient failure into every app dropping
/// its accessibility tree. Nothing is published, and the last known value keeps
/// standing until the broker answers again.
async fn read_screen_reader() -> Option<bool> {
    let client = arlen_config_broker::ConfigBrokerClient::default_socket();
    match client.get().await {
        Ok(state) => Some(state.accessibility.screen_reader),
        Err(e) => {
            log::debug!("accessibility: the broker did not answer ({e})");
            None
        }
    }
}

/// Publish one snapshot.
fn emit(screen_reader: bool) {
    use prost::Message;
    let payload = crate::projects::proto::AccessibilityStatePayload { screen_reader };
    crate::projects::emit_to_event_bus("accessibility.state", payload.encode_to_vec());
}

/// Watch the broker and republish on change. Runs for the life of the session.
pub async fn run_publisher() {
    // Before the first read, so the session publishes the value the person
    // just asked for rather than the one from last time.
    record_login_choice().await;

    let mut last: Option<bool> = None;
    loop {
        if let Some(current) = read_screen_reader().await {
            if last != Some(current) {
                emit(current);
                log::info!("accessibility: screen_reader is now {current}");
                last = Some(current);
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Start the publisher on the shell's runtime.
pub fn start() {
    tauri::async_runtime::spawn(run_publisher());
}
