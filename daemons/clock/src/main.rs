//! `arlen-clockd` - the session daemon that owns the clock.
//!
//! The app is a view: it may be closed at any time and closing it changes
//! nothing, because everything that has to survive that lives here. This
//! process holds the alarms, the timers, the focus session and the stopwatch,
//! writes them down so a restart is not a loss, and serves them on
//! `org.arlen.Clock1`.
//!
//! Every decision it makes is in the library beside it and unit-tested there -
//! when an alarm next rings, what a missed one deserves, which moment the
//! machine should be woken for. What is here is the part that genuinely needs a
//! process: a bus name, a lock around the state, and the timing.
//!
//! **What is not wired yet, said plainly rather than left to be discovered:**
//! ringing (it needs the notification daemon) and asking `org.arlen.Power1` for
//! the wake. Both decisions are built and tested; what is missing is the
//! plumbing, and until it lands this daemon keeps time correctly while awake and
//! says so here rather than in a comment nobody reads.

use std::sync::Arc;

use arlen_clock::missed::LATE_WINDOW_MS;
use arlen_clock::reduce::{self, Command};
use arlen_clock::state::{Alarm, ClockState, FocusConfig};
use arlen_clock::{startup, store};
use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// The bus name the app looks for.
const BUS_NAME: &str = "org.arlen.Clock1";
/// Where the interface lives on it.
const OBJECT_PATH: &str = "/org/arlen/Clock1";

/// Milliseconds since the epoch, the unit every anchor is in.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The daemon's state and the bits it needs to act on a change.
struct Clock {
    state: Arc<Mutex<ClockState>>,
    dir: std::path::PathBuf,
}

impl Clock {
    /// Apply a command, then write the state down if it changed.
    ///
    /// The wake that [`reduce`] works out is dropped for now - see the module
    /// note. It is computed rather than skipped so the day it is wired there is
    /// nothing to remember.
    async fn apply(&self, command: Command) {
        let mut state = self.state.lock().await;
        let effects = reduce::apply(
            &mut state,
            command,
            &chrono::Local,
            // The shared offline city dataset is owned outside the clock
            // (`clock-app.md` §4) and does not exist yet, so a world-clock add
            // resolves to nothing rather than to a city this daemon invented.
            |_| None,
            now_ms(),
        );
        if effects.persist {
            if let Err(e) = store::save(&self.dir, &state) {
                // Losing the write is worth saying loudly: the state in memory
                // is still right, but a restart would now lose it.
                warn!("clock state not written: {e}");
            }
        }
    }

    /// Apply something that happened to the machine rather than something the app
    /// asked for, then write the state down if it changed.
    async fn observe(&self, event: reduce::Event) {
        let mut state = self.state.lock().await;
        let effects = reduce::observe(&mut state, event, now_ms());
        if effects.persist {
            if let Err(e) = store::save(&self.dir, &state) {
                warn!("clock state not written: {e}");
            }
        }
    }

    /// Come back from a sleep: re-derive every anchor, exactly as a restart
    /// does.
    ///
    /// `startup::resume` is not a start-up path that happens to be reusable - it
    /// is the answer to "did anything's moment pass while nothing was watching",
    /// which is the same question after a restart, a crash and a night asleep.
    /// Running only the stopwatch half here would leave an alarm that was due at
    /// 07:00 pointing at a moment already gone.
    async fn resumed(&self) {
        self.observe(reduce::Event::Resumed).await;
        // Asked again here because the power daemon may have been down when this
        // one started, and a wake is the moment the answer is about to matter.
        let capable = wake_capable().await;
        let mut state = self.state.lock().await;
        state.wake_capable = capable;
        let resumed = startup::resume(&mut state, &chrono::Local, LATE_WINDOW_MS, now_ms());
        if !resumed.ring_late.is_empty() {
            warn!(
                alarms = ?resumed.ring_late,
                "alarms came due while the machine slept, and cannot ring yet"
            );
        }
        // Written unconditionally: a machine wakes a handful of times a day, so
        // a comparison to save one file write would buy nothing and could only
        // be wrong.
        if let Err(e) = store::save(&self.dir, &state) {
            warn!("clock state not written after a wake: {e}");
        }
    }
}

/// The daemon that owns suspend, and holds the one capability that lets a timer
/// wake a sleeping machine.
const POWER_SERVICE: &str = "org.arlen.Power1";
/// Where its interface lives.
const POWER_PATH: &str = "/org/arlen/Power1";

/// Whether alarms can wake this machine, asked of the daemon that would do it.
///
/// **Not probed here, deliberately.** `CAP_WAKE_ALARM` is a property of a
/// process, and the process that arms the wake is `arlen-powerd` - so a probe in
/// this daemon would answer about the wrong one and report "cannot wake" on a
/// machine where waking works perfectly. `Power1` probed itself at startup and
/// publishes the answer; asking it is the only way to be right.
///
/// The boolean is read rather than the sentence beside it: a display string is
/// for a person, and matching English across a process boundary is the kind of
/// coupling that breaks silently the day someone improves the wording.
///
/// An unreachable power daemon means `false`, which is the honest direction: the
/// app then says this machine will not be woken, and an alarm that rings anyway
/// is a pleasant surprise rather than a broken promise.
async fn wake_capable() -> bool {
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(proxy) = zbus::Proxy::new(&conn, POWER_SERVICE, POWER_PATH, POWER_SERVICE).await else {
        return false;
    };
    match proxy.get_property::<bool>("WakesMachine").await {
        Ok(capable) => capable,
        Err(e) => {
            warn!("power daemon did not answer whether alarms can wake this machine: {e}");
            false
        }
    }
}

/// What a power event on the bus means to the clock, or `None` for one it does
/// not act on.
///
/// Pure, so the mapping is testable without a bus. Inverting it would stop the
/// stopwatch on waking and start it as the machine goes to sleep, which reads
/// as "the stopwatch is broken" rather than as a wiring mistake.
fn sleep_transition(event_type: &str) -> Option<reduce::Event> {
    match event_type {
        "power.suspend" => Some(reduce::Event::Suspending),
        "power.resume" => Some(reduce::Event::Resumed),
        _ => None,
    }
}

/// The producers whose word the clock takes for "the machine slept".
///
/// The bus stamps `authenticated_origin` from the producer's kernel-attested
/// app id, so a name claimed in the payload cannot land here. `powerd` is the
/// deployed id and `dev.arlen-powerd` the one a cargo-run daemon resolves to.
const SLEEP_PRODUCERS: [&str; 2] = ["powerd", "dev.arlen-powerd"];

/// Whether a sleep event came from the daemon that owns suspend.
///
/// A mismatch is logged and the event is still acted on, deliberately. The bus
/// leaves the origin empty when it could not resolve the peer, and refusing on
/// that would mean a stopwatch silently counting a whole night - the exact bug
/// this subscription exists to fix - while the alternative costs a fold and a
/// restart that preserve the elapsed time and that the person can undo with one
/// press. If the clock ever acts on a power event in a way that is not
/// recoverable, this must become a refusal.
fn sleep_origin_recognised(origin: &str) -> bool {
    SLEEP_PRODUCERS.contains(&origin)
}

/// Follow the machine's sleep transitions for as long as the bus will say.
///
/// **Which suspend types this covers, since the obvious implementation covers
/// only some.** The clock does not read a monotonic clock and infer sleep from
/// a gap: `CLOCK_MONOTONIC` pauses in deep suspend but keeps counting under
/// s2idle, so that route gives "stopwatch paused" on one machine and "stopwatch
/// counted the night" on another. This machine's `/sys/power/mem_sleep` selects
/// `s2idle`, so it is one where that route would be wrong. logind's
/// `PrepareForSleep`, which the power daemon turns into these two events, is
/// broadcast around the sleep *operation* - suspend, hibernate, hybrid-sleep and
/// suspend-then-hibernate alike - and not around whichever mode the kernel then
/// picks, so this path does not vary with the machine.
///
/// If the bus is unreachable the clock keeps working and the stopwatch counts
/// through a sleep; that is worth a loud line rather than a silent difference in
/// behaviour.
async fn watch_sleep(clock: std::sync::Arc<Clock>, socket: String) {
    let consumer = UnixEventConsumer::new(socket);
    let types = vec!["power.suspend".to_string(), "power.resume".to_string()];
    let mut events = match consumer.subscribe(types).await {
        Ok(rx) => rx,
        Err(e) => {
            warn!("no sleep events ({e}); the stopwatch will count time spent asleep");
            return;
        }
    };
    info!("watching for sleep transitions");
    while let Some(event) = events.recv().await {
        let Some(transition) = sleep_transition(&event.r#type) else {
            continue;
        };
        if !sleep_origin_recognised(&event.authenticated_origin) {
            warn!(
                origin = %event.authenticated_origin,
                "a sleep event did not come from the power daemon"
            );
        }
        match transition {
            reduce::Event::Resumed => clock.resumed().await,
            other => clock.observe(other).await,
        }
    }
    warn!("sleep events ended; the stopwatch will count time spent asleep");
}

/// The interface the app talks to.
struct ClockInterface {
    clock: Arc<Clock>,
}

#[zbus::interface(name = "org.arlen.Clock1")]
impl ClockInterface {
    /// Everything the app renders, in one read.
    async fn state(&self) -> String {
        let state = self.clock.state.lock().await;
        serde_json::to_string(&*state).unwrap_or_else(|_| "{}".to_string())
    }

    /// Add or replace an alarm.
    async fn set_alarm(&self, alarm_json: String) -> zbus::fdo::Result<()> {
        let alarm: Alarm = serde_json::from_str(&alarm_json)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("not an alarm: {e}")))?;
        self.clock.apply(Command::SetAlarm(alarm)).await;
        Ok(())
    }

    /// Arm or disarm one.
    async fn toggle_alarm(&self, id: String, enabled: bool) {
        self.clock.apply(Command::ToggleAlarm { id, enabled }).await;
    }

    /// Remove one.
    async fn delete_alarm(&self, id: String) {
        self.clock.apply(Command::DeleteAlarm { id }).await;
    }

    /// Start a countdown. The id is minted here: two apps starting a timer at
    /// the same moment must not collide on one.
    async fn timer_start(&self, duration_ms: i64) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        self.clock
            .apply(Command::TimerStart {
                id: id.clone(),
                duration_ms,
            })
            .await;
        id
    }

    /// Pause or resume one. Takes the wanted state rather than a toggle, so a
    /// repeat is a no-op instead of undoing the first press.
    async fn timer_pause(&self, id: String, paused: bool) {
        self.clock
            .apply(Command::TimerSetPaused { id, paused })
            .await;
    }

    /// Remove one.
    async fn timer_cancel(&self, id: String) {
        self.clock.apply(Command::TimerCancel { id }).await;
    }

    /// Begin a focus session.
    ///
    /// `held` is empty until the notification daemon is asked: a session that
    /// listed what it suppresses without having suppressed anything would be
    /// the dishonesty the design rules out.
    async fn focus_start(&self) {
        self.clock.apply(Command::FocusStart { held: vec![] }).await;
    }

    /// End it early.
    async fn focus_end(&self) {
        self.clock.apply(Command::FocusEnd).await;
    }

    /// Change the focus configuration.
    async fn focus_config(&self, config_json: String) -> zbus::fdo::Result<()> {
        let config: FocusConfig = serde_json::from_str(&config_json)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("not a focus config: {e}")))?;
        self.clock.apply(Command::FocusConfigure(config)).await;
        Ok(())
    }

    /// Start or resume the stopwatch.
    async fn stopwatch_start(&self) {
        self.clock.apply(Command::StopwatchStart).await;
    }

    /// Pause it.
    async fn stopwatch_pause(&self) {
        self.clock.apply(Command::StopwatchPause).await;
    }

    /// Record a lap.
    async fn stopwatch_lap(&self) {
        self.clock.apply(Command::StopwatchLap).await;
    }

    /// Back to zero.
    async fn stopwatch_reset(&self) {
        self.clock.apply(Command::StopwatchReset).await;
    }

    /// Show a city, by its id in the shared dataset.
    async fn world_add(&self, id: String) {
        self.clock.apply(Command::WorldAdd { id }).await;
    }

    /// Stop showing one.
    async fn world_remove(&self, id: String) {
        self.clock.apply(Command::WorldRemove { id }).await;
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Some(dir) = store::state_dir() else {
        // Nowhere to keep alarms is not something to paper over: a clock that
        // silently forgets everything on restart is worse than one that refuses
        // to start and says why.
        eprintln!("arlen-clockd: no per-user state directory (XDG_STATE_HOME and HOME unset)");
        std::process::exit(1);
    };

    let mut state = match store::load(&dir) {
        store::Loaded::Kept(state) => state,
        store::Loaded::Fresh => {
            info!("no previous clock state, starting empty");
            ClockState::default()
        }
        store::Loaded::Unreadable { path, reason } => {
            warn!(
                "clock state could not be read ({reason}); kept at {} and starting empty",
                path.display()
            );
            ClockState::default()
        }
    };

    // Bring the anchors up to date before anything can read them, so the first
    // `State` call answers about now rather than about whenever the daemon last
    // ran.
    state.wake_capable = wake_capable().await;
    info!(
        wake_capable = state.wake_capable,
        "asked the power daemon about waking"
    );

    let resumed = startup::resume(&mut state, &chrono::Local, LATE_WINDOW_MS, now_ms());
    if !resumed.ring_late.is_empty() {
        // Not rung: the notification daemon is not wired yet. Recorded so the
        // gap is visible in the log rather than being a silent nothing.
        warn!(
            alarms = ?resumed.ring_late,
            "alarms were missed while the daemon was not running, and cannot ring yet"
        );
    }
    if let Err(e) = store::save(&dir, &state) {
        warn!("clock state not written after resume: {e}");
    }

    let clock = Arc::new(Clock {
        state: Arc::new(Mutex::new(state)),
        dir,
    });

    tokio::spawn(watch_sleep(
        Arc::clone(&clock),
        os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock")
            .to_string_lossy()
            .into_owned(),
    ));

    let conn = match zbus::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJECT_PATH, ClockInterface { clock }))
    {
        Ok(builder) => match builder.build().await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("arlen-clockd: cannot serve {BUS_NAME}: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("arlen-clockd: cannot build the {BUS_NAME} connection: {e}");
            std::process::exit(1);
        }
    };
    info!("clock daemon serving {BUS_NAME}");

    // Held for the process's life: dropping the connection drops the name.
    let _conn = conn;
    match tokio::signal::ctrl_c().await {
        Ok(()) => info!("clock daemon shutting down"),
        Err(e) => warn!("shutdown signal failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inverted, this would stop the stopwatch on waking and start it as the
    /// machine goes to sleep.
    #[test]
    fn each_power_event_means_the_transition_it_names() {
        assert_eq!(
            sleep_transition("power.suspend"),
            Some(reduce::Event::Suspending)
        );
        assert_eq!(
            sleep_transition("power.resume"),
            Some(reduce::Event::Resumed)
        );
    }

    /// The subscription is by prefix on the bus, so a neighbouring power event
    /// can arrive here and must not be read as a sleep.
    #[test]
    fn no_other_power_event_moves_the_stopwatch() {
        for other in [
            "power.state",
            "power.profile_changed",
            "power.battery_low",
            "window.focused",
            "",
        ] {
            assert_eq!(sleep_transition(other), None, "{other} is not a sleep");
        }
    }

    /// The two ids the power daemon actually resolves to, deployed and in a
    /// cargo-run stack. A typo here would log a warning on every real sleep.
    #[test]
    fn the_power_daemons_own_ids_are_recognised() {
        assert!(sleep_origin_recognised("powerd"));
        assert!(sleep_origin_recognised("dev.arlen-powerd"));
    }

    /// An unresolved origin is not the power daemon, so it is worth saying -
    /// the event is still acted on, which is the caller's decision and is
    /// argued where that happens.
    #[test]
    fn anything_else_is_not_recognised_including_an_unresolved_peer() {
        for origin in ["", "unknown", "settings", "arlen-powerd", "powerd "] {
            assert!(
                !sleep_origin_recognised(origin),
                "{origin:?} is not the power daemon"
            );
        }
    }
}
