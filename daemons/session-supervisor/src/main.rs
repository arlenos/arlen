//! The session supervisor: starts the per-user Arlen daemons and keeps their
//! identity registration current across restarts.
//!
//! The decision half ([`arlen_session_supervisor::supervise`]) is pure and tested.
//! This binary is the two seams around it - the user manager over D-Bus, and the
//! identity broker - plus the loop that runs a round and waits.
//!
//! **Why a loop and not one pass.** A registration names a pid, and `Restart=` is
//! on for most of these units, so a crash gives a NEW MainPID within about a
//! second and the old registration is then a record of a process that no longer
//! exists. One pass at login would be correct for exactly as long as nothing
//! restarted. The round is idempotent, so repeating it costs two property reads
//! per unit and fixes that.
//!
//! **Why polling and not signals.** systemd will emit `PropertiesChanged` for
//! MainPID, and subscribing is the tidier design. It is also a second thing that
//! can silently stop delivering, and the cost of missing an edge here is a daemon
//! that authenticates nobody until the next event. A poll cannot miss an edge, it
//! can only be late by the interval - which is the failure mode worth having.
//!
//! **A run outside a session says so and exits non-zero.** Not a stub that exits
//! early: it reports per unit what it would have supervised and why it could not,
//! because a component wired to nothing should leave a report a reader can act on.

use std::time::Duration;

use arlen_permissions::unit_identity::{app_id_for_user_unit, enrolled_user_units};
use arlen_session_supervisor::broker::BrokerRegistrar;
use arlen_session_supervisor::supervise::{round, Action, Registered, Systemd};
use arlen_session_supervisor::systemd::SystemdBus;

/// How long to wait between rounds.
///
/// A restart is visible within about a second, so this is the window in which a
/// restarted daemon is registered under its dead pid - it authenticates nobody
/// until the next round, which is fail-closed. Short enough that a user does not
/// notice, long enough that the traffic is nothing: two property reads per unit.
const ROUND_INTERVAL: Duration = Duration::from_secs(5);

/// The systemd seam, absent. Every call fails with what is missing rather than a
/// generic error, so the run says which half is unbuilt.
struct NoSystemd(String);

impl Systemd for NoSystemd {
    fn start(&self, _unit: &str) -> Result<(), String> {
        Err(self.0.clone())
    }
    fn main_pid(&self, _unit: &str) -> Result<u32, String> {
        Err(self.0.clone())
    }
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // `info` for THIS daemon, not the usual `warn`: its entire output is
                // one line per registration CHANGE, and the steady state is at debug.
                // At warn the boot of 13 Aug showed only the one unit that failed and
                // nothing about the sixteen that worked - which reads as a component
                // that did almost nothing, when it had done its job.
                //
                // The level was a blanket until 22 Aug, which gave every dependency
                // in the process the same `info` and was never what the paragraph
                // above asked for. Naming the crate keeps the argument and drops the
                // rest.
                .unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("warn,arlen_session_supervisor=info")
                }),
        )
        .init();

    // The units this supervisor is responsible for, from the one table that names
    // them - so the binary and the resolver cannot disagree about the set.
    let units: Vec<(&str, &str)> = enrolled_user_units()
        .filter_map(|u| app_id_for_user_unit(u).map(|id| (u, id)))
        .collect();

    let registrar = BrokerRegistrar::at_default_socket();
    let systemd = match SystemdBus::session() {
        Ok(bus) => bus,
        Err(e) => {
            // One round against a refusing seam, so the output names every unit
            // that went unsupervised and the one reason none of them could be.
            let mut registered = Registered::new();
            let refusing = NoSystemd(e.clone());
            for (unit, outcome) in round(&units, &refusing, &registrar, &mut registered) {
                tracing::warn!(unit, "not supervised: {}", outcome.unwrap_err());
            }
            eprintln!(
                "arlen-session-supervisor: {} unit(s) went unsupervised and no identity was \
                 registered - {e}",
                units.len()
            );
            return std::process::ExitCode::FAILURE;
        }
    };

    tracing::info!(units = units.len(), "supervising");
    let mut registered = Registered::new();
    loop {
        for (unit, outcome) in round(&units, &systemd, &registrar, &mut registered) {
            match outcome {
                // Unchanged is the steady state and every round produces one per
                // unit, so it stays at debug: a log that repeats the same line
                // every five seconds is one nobody reads the rest of.
                Ok(Action::Unchanged) => tracing::debug!(unit, "unchanged"),
                Ok(Action::Register { pid, replacing }) => {
                    tracing::info!(unit, pid, ?replacing, "identity registered")
                }
                Ok(Action::NotRunning) => tracing::debug!(unit, "not running"),
                // Debug, not warn: an entry absent from this image is expected on
                // every production boot (the verify probe), and a warning per
                // round per absent unit buries the ones that mean something.
                Ok(Action::NotInstalled) => tracing::debug!(unit, "not installed on this image"),
                Err(e) => tracing::warn!(unit, "not supervised: {e}"),
            }
        }
        std::thread::sleep(ROUND_INTERVAL);
    }
}
