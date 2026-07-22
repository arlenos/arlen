//! Sleep transitions: logind `PrepareForSleep` -> `power.suspend` / `power.resume`.
//!
//! `contracts/event/proto/event.proto` specifies both as coarse power
//! transitions, promoted to the Knowledge Graph as local provenance ("the
//! machine slept here"). Nothing published them: the daemon emitted
//! `power.state`, the battery-level crossings and `power.profile_changed`, so
//! the sleep half of that contract was simply missing from the wire.
//!
//! logind broadcasts `PrepareForSleep(true)` immediately BEFORE the machine
//! sleeps and `PrepareForSleep(false)` after it wakes. This module only
//! OBSERVES that signal and emits; it never asks logind to sleep - that is
//! [`crate::logind::perform`], behind the PWR-R7 capability check. Watching a
//! broadcast grants no new authority, so this path needs no gate of its own.

use tracing::{debug, warn};

/// logind's manager, whose `PrepareForSleep` broadcast this watches.
const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";

/// The transition event type for a `PrepareForSleep` payload.
///
/// `true` is the pre-sleep broadcast, `false` the post-wake one - so the signal
/// carries BOTH transitions and the boolean is the only thing distinguishing
/// them. Pure, so the mapping is testable without logind.
pub fn sleep_event_type(starting: bool) -> &'static str {
    if starting {
        "power.suspend"
    } else {
        "power.resume"
    }
}

/// Subscribe to `PrepareForSleep` and hand each transition's event type to
/// `emit`. Runs until the signal stream ends (the bus dropped), so the caller
/// spawns it and reconnects with the rest of the daemon's bus handling.
///
/// Errors while subscribing are returned; a malformed signal body is skipped
/// rather than ending the watch, since one bad message must not stop the
/// daemon noticing every later sleep.
pub async fn watch<F, Fut>(conn: &zbus::Connection, mut emit: F) -> zbus::Result<()>
where
    F: FnMut(&'static str) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use futures_util::StreamExt;

    let manager =
        zbus::Proxy::new(conn, LOGIND_SERVICE, LOGIND_PATH, LOGIND_MANAGER).await?;
    let mut signals = manager.receive_signal("PrepareForSleep").await?;
    debug!("watching logind PrepareForSleep");

    while let Some(msg) = signals.next().await {
        match msg.body().deserialize::<bool>() {
            Ok(starting) => emit(sleep_event_type(starting)).await,
            Err(e) => warn!("PrepareForSleep body not a bool: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boolean_selects_the_transition() {
        // logind sends `true` BEFORE sleeping and `false` after waking, so the
        // mapping must not be inverted - swapping these would record every wake
        // as a sleep in the graph's local provenance.
        assert_eq!(sleep_event_type(true), "power.suspend");
        assert_eq!(sleep_event_type(false), "power.resume");
    }

    #[test]
    fn both_types_are_the_ones_the_proto_specifies() {
        // The proto's PowerTransitionPayload comment enumerates the accepted
        // event types; a typo here would publish an event nothing consumes.
        for t in [sleep_event_type(true), sleep_event_type(false)] {
            assert!(
                ["power.suspend", "power.resume"].contains(&t),
                "{t} is not a specified power transition"
            );
        }
    }
}
