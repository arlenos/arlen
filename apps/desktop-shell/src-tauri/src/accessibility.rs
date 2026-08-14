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
/// ABSENT MEANS "NOBODY OPERATED THE TOGGLE", NOT "OFF". The greeter sets the
/// variable only when somebody actually worked the switch at that login, so an
/// untouched login screen leaves whatever the person already had. Their own
/// config must not be overwritten by a default they never chose - and the login
/// screen has its own remembered default, which arriving on screen says nothing
/// about the person walking up to it.
///
/// A `0` IS a statement: they reached over and turned it off, which carries the
/// same weight as turning it on.
///
/// The write is what makes this stick: the greeter cannot read the broker back
/// (different uid, and before login no user is chosen), but the SHELL reads it
/// every session. So somebody turns it on at the login screen once, the session
/// records it, and every later session already has it without the login screen
/// being involved at all.
async fn record_login_choice() {
    let Some(chosen) = login_choice(std::env::var(HANDOFF_ENV).ok().as_deref()) else {
        return;
    };
    let client = arlen_config_broker::ConfigBrokerClient::default_socket();
    // Read first: if it already says this, there is nothing to record, and a
    // write that changes nothing is a write that can still fail.
    match client.get().await {
        Ok(state) if state.accessibility.screen_reader == chosen => return,
        Ok(_) => {}
        Err(e) => {
            log::warn!("accessibility: cannot record the login choice, broker unreadable ({e})");
            return;
        }
    }
    let wanted = arlen_config_broker::Accessibility { screen_reader: chosen };
    match client.set_accessibility(wanted).await {
        Ok(()) => log::info!("accessibility: recorded the login choice, screen_reader {chosen}"),
        // Worth a warning rather than silence: the person will find it off again
        // next login and have no idea why.
        Err(e) => log::warn!("accessibility: could not record the login choice ({e})"),
    }
}

/// What the login screen said, if it said anything.
///
/// Three-state, and the middle state is the one the whole feature turns on:
/// `None` means nobody worked the toggle, so the session keeps whatever that
/// user's own config holds. The greeter has its own remembered default, and that
/// default arriving on screen is a fact about the door rather than a decision by
/// the person walking through it - so it must not overwrite a preference they
/// set inside their session.
///
/// A value neither side writes is also `None`. Guessing a boolean out of an
/// unexpected string would be inventing somebody's accessibility setting, and
/// the safe direction is to say nothing.
fn login_choice(raw: Option<&str>) -> Option<bool> {
    match raw {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::login_choice;

    #[test]
    fn an_untouched_login_says_nothing() {
        // The bound that keeps this intent and not a side effect. If this ever
        // becomes `Some(false)`, a person with the reader on inside their
        // session loses it by walking past a greeter nobody touched.
        assert_eq!(login_choice(None), None);
    }

    #[test]
    fn both_answers_travel() {
        assert_eq!(login_choice(Some("1")), Some(true));
        // Switching it OFF at the login screen is as deliberate as switching it
        // on, so it must not collapse into the untouched case.
        assert_eq!(login_choice(Some("0")), Some(false));
    }

    #[test]
    fn an_unexpected_value_is_not_a_guess() {
        // Neither side writes these. Reading `true` as on would be inventing a
        // setting from a string nobody agreed to.
        for raw in ["true", "yes", "on", "", " 1", "1 ", "01"] {
            assert_eq!(login_choice(Some(raw)), None, "{raw:?} must say nothing");
        }
    }
}
