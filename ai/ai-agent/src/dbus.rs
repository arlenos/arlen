//! D-Bus interface object for `org.arlen.AIAgent1`.
//!
//! Exposes the agent's *live loop status* as a read-only property, so a caller
//! (the harness agent dashboard, the TopBar) can see what the running daemon is
//! doing right now: waiting for a trigger, or handling one. This is the
//! "telephone" surface from the design (`KG = blackboard (pull), notifications
//! = mailbox (push), D-Bus = telephone (live/blocking)`): D-Bus carries live
//! state, not pull data.
//!
//! It deliberately does NOT re-expose the static behaviour catalogue (name,
//! kind, enabled, provenance, reads). That set is pull data the harness already
//! reads in-process through `arlen_ai_agent::loader::load_configured`
//! (`ai_behaviours`), and it is identical whether read there or here because the
//! daemon tracks no extra per-behaviour state. Duplicating it over D-Bus would
//! add a second source of the same truth with no new information.
//!
//! The interface is served on the same connection that owns the well-known name
//! `org.arlen.AIAgent1` (see `main.rs`), so the existing `ai-proxy` peer-auth
//! contract is preserved: the connection that owns the name is also the one that
//! forwards LLM traffic through the proxy. Registering an interface object does
//! not add a second connection or change name ownership.
//!
//! Status changes are not announced with `PropertiesChanged`: the value flips on
//! every dispatched event, and the harness polls its read surfaces today (the
//! activity timeline refreshes rather than subscribes). The property is annotated
//! `emits_changed_signal = "false"` so the contract is honest about that. A live
//! subscribe path (idle/thinking/acting split + a change signal) is the follow-up
//! that lands when the harness moves from poll to subscribe.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use zbus::interface;

/// The D-Bus object path the interface is registered under.
pub const AGENT_OBJECT_PATH: &str = "/org/arlen/AIAgent1";

/// The agent's live loop status: what the running daemon is doing right now.
///
/// This is the honest resolution the dispatch loop can report without reaching
/// into the engine: `Subscribing` before the Event Bus subscription is
/// established (the daemon is up but cannot yet receive triggers), `Idle` once
/// it is waiting for the next trigger, and `Busy` while a dispatched event is
/// being handled (for a `kind: agent` behaviour that covers the whole bounded
/// loop). The `Subscribing` state matters for honesty: without it a poller
/// would read `idle` during an Event Bus outage and mistake an unreachable
/// daemon for a healthy waiting one. A finer `thinking` (provider call) vs
/// `acting` (executor write) split needs engine-internal hooks and is a
/// follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStatus {
    /// Up, but the Event Bus subscription is not yet established; no trigger
    /// can be received yet.
    Subscribing,
    /// Subscribed and waiting for the next trigger; no behaviour is running.
    Idle,
    /// Handling a dispatched event (matching, screening, or running a loop).
    Busy,
}

impl LoopStatus {
    /// The wire string a caller reads from the `Status` property.
    pub fn as_str(self) -> &'static str {
        match self {
            LoopStatus::Subscribing => "subscribing",
            LoopStatus::Idle => "idle",
            LoopStatus::Busy => "busy",
        }
    }

    /// Reconstruct from the atomic encoding, defaulting to `Subscribing` for any
    /// unexpected byte so a torn read fails toward "not yet ready" rather than a
    /// healthy-looking `idle`.
    fn from_u8(v: u8) -> Self {
        match v {
            1 => LoopStatus::Idle,
            2 => LoopStatus::Busy,
            _ => LoopStatus::Subscribing,
        }
    }

    /// The atomic encoding.
    fn to_u8(self) -> u8 {
        match self {
            LoopStatus::Subscribing => 0,
            LoopStatus::Idle => 1,
            LoopStatus::Busy => 2,
        }
    }
}

/// Shared live-status cell, written by the dispatch loop and read by the D-Bus
/// property getter. A single atomic byte: the only states are `Idle`/`Busy`, the
/// writes are point updates with no cross-field invariant, and the reader only
/// needs the latest value, so no lock is warranted.
pub type StatusHandle = Arc<AtomicU8>;

/// Create a status handle initialised to `Subscribing` (the daemon is up but
/// has not yet established its Event Bus subscription, so it cannot receive
/// triggers; reporting `idle` here would hide that).
pub fn new_status_handle() -> StatusHandle {
    Arc::new(AtomicU8::new(LoopStatus::Subscribing.to_u8()))
}

/// Publish the current loop status for the D-Bus property to read.
pub fn set_status(handle: &StatusHandle, status: LoopStatus) {
    handle.store(status.to_u8(), Ordering::Relaxed);
}

/// Read the current loop status.
pub fn load_status(handle: &StatusHandle) -> LoopStatus {
    LoopStatus::from_u8(handle.load(Ordering::Relaxed))
}

/// The `org.arlen.AIAgent1` D-Bus interface object.
///
/// Holds the shared status cell so the property always reflects the live loop
/// state, not a startup snapshot.
pub struct AgentInterface {
    /// Live loop status, updated by the dispatch loop.
    pub status: StatusHandle,
}

#[interface(name = "org.arlen.AIAgent1")]
impl AgentInterface {
    /// The agent's live loop status: `"subscribing"` before it can receive
    /// triggers, `"idle"` when waiting for one, `"busy"` when handling one.
    /// Read-only.
    ///
    /// `emits_changed_signal = "false"`: the value flips on every dispatched
    /// event and callers poll it; no `PropertiesChanged` is sent.
    #[zbus(property(emits_changed_signal = "false"))]
    async fn status(&self) -> String {
        load_status(&self.status).as_str().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_handle_reads_subscribing_not_idle() {
        // The honesty contract: before the daemon subscribes, a poller must not
        // see a healthy-looking "idle".
        let handle = new_status_handle();
        assert_eq!(load_status(&handle), LoopStatus::Subscribing);
        assert_eq!(load_status(&handle).as_str(), "subscribing");
    }

    #[test]
    fn set_then_load_round_trips() {
        let handle = new_status_handle();
        set_status(&handle, LoopStatus::Busy);
        assert_eq!(load_status(&handle), LoopStatus::Busy);
        assert_eq!(load_status(&handle).as_str(), "busy");
        set_status(&handle, LoopStatus::Idle);
        assert_eq!(load_status(&handle), LoopStatus::Idle);
        assert_eq!(load_status(&handle).as_str(), "idle");
        set_status(&handle, LoopStatus::Subscribing);
        assert_eq!(load_status(&handle), LoopStatus::Subscribing);
    }

    #[test]
    fn an_unknown_byte_decodes_to_subscribing() {
        // Fail toward "not yet ready", never a healthy-looking idle.
        let handle: StatusHandle = Arc::new(AtomicU8::new(200));
        assert_eq!(load_status(&handle), LoopStatus::Subscribing);
    }

    #[test]
    fn the_handle_is_shared_not_copied() {
        let handle = new_status_handle();
        let clone = Arc::clone(&handle);
        set_status(&clone, LoopStatus::Busy);
        // A write through one Arc is visible through the other: it is one cell.
        assert_eq!(load_status(&handle), LoopStatus::Busy);
    }
}
