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
//! consumer.
//!
//! **It waits for the display rather than assuming it arrived first.** The
//! daemon starts at `default.target`, the compositor comes up separately, and
//! for the whole life of this code the two lost that race: a 12 Aug boot
//! journal reads `idle policy inactive: no wayland display`, once, at startup,
//! and nothing after - so no idle blank and no idle suspend ever armed on a
//! real boot. Start order cannot fix it (display readiness is not a unit
//! state, and an `After=` would only narrow the window), and neither can a
//! retry on its own: `connect_to_env` reads `$WAYLAND_DISPLAY` from the
//! process environment, which is fixed at exec, and powerd's unit never had
//! it set. So [`spawn`] DISCOVERS the socket in `$XDG_RUNTIME_DIR` and keeps
//! looking until one appears.

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
    /// The discovered socket exists but would not accept a connection.
    #[error("wayland socket: {0}")]
    Socket(std::io::Error),
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

/// How often to look for the display while it is not there yet. Slow enough to
/// be free on a machine that will never have a compositor (a headless install
/// keeps this thread asleep), quick enough that a normal session arms within a
/// couple of seconds of the bar appearing.
const DISCOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// The Wayland socket to connect to, or `None` while none exists.
///
/// `$WAYLAND_DISPLAY` first, because a session that exports it has said which
/// display it means. Failing that, the lowest-numbered `wayland-N` in
/// `$XDG_RUNTIME_DIR` - which is where the compositor puts it, and the reason
/// this function exists at all: powerd is started by systemd without that
/// variable, so reading the environment alone can only ever fail.
///
/// The `.lock` companion file is skipped, and so is anything that is not a
/// socket: a stale regular file left by a crashed compositor would otherwise be
/// picked as a display and connected to forever.
pub fn discover_display(runtime_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    if let Ok(name) = std::env::var("WAYLAND_DISPLAY") {
        if !name.is_empty() {
            let p = std::path::Path::new(&name);
            let full = if p.is_absolute() { p.to_path_buf() } else { runtime_dir.join(p) };
            if is_socket(&full) {
                return Some(full);
            }
        }
    }
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(runtime_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
        })
        .filter(|p| is_socket(p))
        .collect();
    found.sort();
    found.into_iter().next()
}

fn is_socket(p: &std::path::Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(p).is_ok_and(|m| m.file_type().is_socket())
}

/// Spawn the idle-notify client on a dedicated thread, forwarding
/// [`IdleSignal`]s over `tx`. Returns the join handle. Registers one
/// notification per stage; an empty `stages` (idle policy fully disabled)
/// spawns nothing and returns `None`.
///
/// The thread outlives any single connection: it waits for a display, arms,
/// and re-arms if the compositor goes away and comes back. A restarted
/// compositor is the same case as one that has not started yet, so there is
/// only one path and it is the one that gets exercised on every boot.
pub fn spawn(stages: Vec<IdleStage>, tx: Sender<IdleSignal>) -> Option<std::thread::JoinHandle<()>> {
    if stages.is_empty() {
        return None;
    }
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/run/user/1000"));
    Some(std::thread::spawn(move || {
        // Log the wait once, not every two seconds. A daemon that repeats the
        // same line forever trains its reader to skip it, and this one has a
        // real thing to say when the state changes.
        let mut announced_wait = false;
        loop {
            let Some(socket) = discover_display(&runtime_dir) else {
                if !announced_wait {
                    tracing::info!(
                        dir = %runtime_dir.display(),
                        "idle policy waiting for a wayland display"
                    );
                    announced_wait = true;
                }
                std::thread::sleep(DISCOVERY_INTERVAL);
                continue;
            };
            announced_wait = false;
            match std::os::unix::net::UnixStream::connect(&socket)
                .map_err(IdleClientError::Socket)
                .and_then(|s| Connection::from_socket(s).map_err(IdleClientError::from))
            {
                Ok(conn) => {
                    tracing::info!(
                        socket = %socket.display(),
                        stages = stages.len(),
                        "idle policy armed"
                    );
                    if let Err(e) = run(conn, &stages, tx.clone()) {
                        tracing::info!("idle policy stopped, will re-arm: {e}");
                    }
                }
                Err(e) => tracing::info!("idle policy could not connect, retrying: {e}"),
            }
            std::thread::sleep(DISCOVERY_INTERVAL);
        }
    }))
}

/// Connect, bind the seat + idle notifier, register a notification per stage,
/// and run the event loop forwarding signals. Blocking; returns on a lost
/// connection or a missing notifier.
fn run(
    conn: Connection,
    stages: &[IdleStage],
    tx: Sender<IdleSignal>,
) -> Result<(), IdleClientError> {
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
    /// A directory with no display in it yet is the state every boot starts in,
    /// and the answer has to be "not yet" rather than anything that looks like a
    /// display - the caller loops on this.
    #[test]
    fn an_empty_runtime_dir_has_no_display() {
        let d = tempfile::tempdir().unwrap();
        assert!(super::discover_display(d.path()).is_none());
    }

    /// The compositor's socket is found without `$WAYLAND_DISPLAY`, which is the
    /// whole point: powerd is started by systemd and never had that variable.
    #[test]
    fn a_wayland_socket_is_found_by_scanning() {
        let d = tempfile::tempdir().unwrap();
        let sock = d.path().join("wayland-1");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert_eq!(super::discover_display(d.path()), Some(sock));
    }

    /// The lock file sits right next to the socket and sorts before it. Taking it
    /// would mean connecting to a regular file on every session.
    #[test]
    fn the_lock_file_is_not_a_display() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("wayland-0.lock"), b"").unwrap();
        assert!(super::discover_display(d.path()).is_none());
    }

    /// A crashed compositor can leave a plain file where its socket was. It is
    /// named exactly like a display and is not one, so the type is what decides -
    /// otherwise powerd would pick it and retry against it for the whole session.
    #[test]
    fn a_stale_regular_file_is_not_a_display() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("wayland-0"), b"stale").unwrap();
        let sock = d.path().join("wayland-1");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert_eq!(super::discover_display(d.path()), Some(sock));
    }

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
