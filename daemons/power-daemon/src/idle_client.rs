//! PWR-R3 idle-notify client: the compositor side of the idle policy.
//!
//! The compositor (cosmic-comp) implements `ext-idle-notify-v1`: a client
//! registers a notification with a timeout, and the compositor fires
//! `idled` when the seat has had no input for that long and `resumed` on
//! the next input. This client registers ONE notification per resolved
//! [`IdleStage`](crate::idle::IdleStage) (its timeout in ms) and forwards
//! each `idled`/`resumed` as an [`IdleSignal`] over a channel to the async
//! side, which runs the stage's action ([`crate::idle::IdleAction`]).
//!
//! It runs a blocking Wayland event loop, so the daemon drives it on a
//! dedicated thread ([`spawn`]); the thread forwards signals to a tokio
//! consumer. A missing notifier global (a compositor without the protocol)
//! or a lost display connection ends the loop cleanly - the session then
//! simply has no idle policy, which the caller logs.

use tokio::sync::mpsc::Sender;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use crate::idle::IdleStage;

/// A seat-idle transition the compositor reported for one registered stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleSignal {
    /// Index into the resolved stage list this notification was created for.
    pub stage: usize,
    /// `false` = the seat just went idle (fire the action); `true` = the
    /// seat resumed (undo a reversible action).
    pub resumed: bool,
}

/// A failure bringing up the idle-notify client. Every variant simply means
/// the session gets no idle policy (the caller logs and carries on); none is
/// fatal to the daemon.
#[derive(Debug, thiserror::Error)]
pub enum IdleClientError {
    /// No Wayland display to connect to (`$WAYLAND_DISPLAY` unset / no socket).
    #[error("no wayland display: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// The registry could not be enumerated.
    #[error("wayland globals: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    /// The compositor exposes no `wl_seat` (nothing to track idle on).
    #[error("no wl_seat")]
    NoSeat,
    /// The compositor does not implement `ext-idle-notify-v1`.
    #[error("compositor has no ext-idle-notify-v1")]
    NoIdleSupport,
    /// The event loop failed (usually the compositor went away).
    #[error("wayland dispatch: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
}

/// The dispatch state: just the channel the notification events are pushed to.
struct State {
    tx: Sender<IdleSignal>,
}

// The registry is enumerated up front by `registry_queue_init`; no dynamic
// global handling is needed.
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// The seat + notifier carry no events this client acts on.
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// Each notification's user-data is its stage index; an idled/resumed event
// forwards an `IdleSignal` for that stage.
impl Dispatch<ExtIdleNotificationV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        stage: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let resumed = match event {
            ext_idle_notification_v1::Event::Idled => false,
            ext_idle_notification_v1::Event::Resumed => true,
            _ => return,
        };
        // The compositor thread is not a tokio context, so a blocking send is
        // correct; the async consumer drains promptly. A closed channel
        // (consumer gone) just drops the signal.
        let _ = state.tx.blocking_send(IdleSignal {
            stage: *stage,
            resumed,
        });
    }
}

/// Spawn the idle-notify client on a dedicated thread, forwarding
/// [`IdleSignal`]s over `tx`. Returns the join handle. Registers one
/// notification per stage; an empty `stages` (idle policy fully disabled)
/// spawns nothing and returns `None`.
pub fn spawn(stages: Vec<IdleStage>, tx: Sender<IdleSignal>) -> Option<std::thread::JoinHandle<()>> {
    if stages.is_empty() {
        return None;
    }
    Some(std::thread::spawn(move || {
        if let Err(e) = run(&stages, tx) {
            tracing::info!("idle policy inactive: {e}");
        }
    }))
}

/// Connect, bind the seat + idle notifier, register a notification per stage,
/// and run the event loop forwarding signals. Blocking; returns on a lost
/// connection or a missing notifier.
fn run(stages: &[IdleStage], tx: Sender<IdleSignal>) -> Result<(), IdleClientError> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();

    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| IdleClientError::NoSeat)?;
    let notifier: ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=2, ())
        .map_err(|_| IdleClientError::NoIdleSupport)?;

    // One notification per stage; its user-data is the stage index so the
    // event handler knows which action to run. Held for the loop's life so
    // the compositor keeps tracking them.
    let _notifications: Vec<ExtIdleNotificationV1> = stages
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let timeout_ms = s.after_secs.saturating_mul(1000);
            notifier.get_idle_notification(timeout_ms, &seat, &qh, i)
        })
        .collect();

    tracing::info!(stages = stages.len(), "idle policy active");
    let mut state = State { tx };
    loop {
        queue.blocking_dispatch(&mut state)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idle::{IdleAction, IdleStage};

    /// Runtime verify against a LIVE compositor that implements
    /// `ext-idle-notify-v1` (the dev/CI host runs one). Registers a 1-second
    /// stage and asserts the `idled` signal arrives, exercising the whole
    /// connect -> bind seat + notifier -> get_idle_notification -> dispatch
    /// path that no unit test can reach. Read-only: the harness only receives
    /// the signal (no executor runs), so it does not touch the live session.
    ///
    /// `#[ignore]d`: needs `$WAYLAND_DISPLAY`, an idle seat (no input during
    /// the wait) and the protocol, so it is a manual / capable-host verify,
    /// not part of the default suite.
    #[tokio::test]
    #[ignore = "needs a live ext-idle-notify-v1 compositor and an idle seat"]
    async fn idled_fires_against_a_live_compositor() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let stages = vec![IdleStage {
            after_secs: 1,
            action: IdleAction::Blank,
        }];
        let _handle = spawn(stages, tx).expect("non-empty stages spawn a client");

        let sig = tokio::time::timeout(std::time::Duration::from_secs(8), rx.recv())
            .await
            .expect("no idle signal within 8s (is the seat idle and the compositor ext-idle-notify-capable?)")
            .expect("the client channel closed before firing");
        assert_eq!(sig.stage, 0, "the single stage's index");
        assert!(!sig.resumed, "expected an idled (not resumed) signal");
    }
}
