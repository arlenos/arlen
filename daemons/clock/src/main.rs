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
//! ringing (it needs the notification daemon), the `power.suspend` /
//! `power.resume` subscription that stops the stopwatch counting a closed lid,
//! and asking `org.arlen.Power1` for the wake. The decisions for all three are
//! built and tested; what is missing is the plumbing, and until it lands this
//! daemon keeps time correctly while awake and says so here rather than in a
//! comment nobody reads.

use std::sync::Arc;

use arlen_clock::missed::LATE_WINDOW_MS;
use arlen_clock::reduce::{self, Command};
use arlen_clock::state::{Alarm, ClockState, FocusConfig};
use arlen_clock::{startup, store};
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
