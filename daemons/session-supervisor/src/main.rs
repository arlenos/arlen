//! The session supervisor, running its real loop against stand-ins that refuse.
//!
//! The decision half ([`arlen_session_supervisor::supervise`]) is built and
//! tested. The systemd client and the pidfd registration are not, so this runs
//! the loop it will always run, over the units it will always supervise, against
//! seams that fail closed - the same shape as `DeniedGraph`, `DeniedBroker` and
//! `DenyUnlessEmpty` elsewhere in the tree.
//!
//! That is deliberately not a stub that exits early. A component wired to nothing
//! should say what it would have done and why it could not, per unit, rather than
//! print one line about itself: the first is a report a reader can act on, the
//! second is the silent-success shape the identity work exists to remove. It still
//! exits non-zero, because a supervisor that supervised nothing did not succeed.

use arlen_permissions::unit_identity::{app_id_for_user_unit, enrolled_user_units};
use arlen_session_supervisor::supervise::{round, Registered, Registrar, Systemd};

/// The systemd seam, absent. Every call fails with what is missing rather than a
/// generic error, so the run says which half is unbuilt.
struct NoSystemd;

impl Systemd for NoSystemd {
    fn start(&self, _unit: &str) -> Result<(), String> {
        Err("no systemd client: the D-Bus seam is not wired".to_string())
    }
    fn main_pid(&self, _unit: &str) -> Result<u32, String> {
        Err("no systemd client: the D-Bus seam is not wired".to_string())
    }
}

/// The broker seam, absent. Fail-closed: a registrar that cannot register must
/// never report that it did.
struct NoRegistrar;

impl Registrar for NoRegistrar {
    fn register(&self, _pid: u32, _app_id: &str) -> Result<(), String> {
        Err("no identity broker client: registration is not wired".to_string())
    }
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    // The units this supervisor is responsible for, from the one table that names
    // them - so the binary and the resolver cannot disagree about the set.
    let units: Vec<(&str, &str)> = enrolled_user_units()
        .filter_map(|u| app_id_for_user_unit(u).map(|id| (u, id)))
        .collect();

    let mut registered = Registered::new();
    let outcomes = round(&units, &NoSystemd, &NoRegistrar, &mut registered);
    for (unit, outcome) in &outcomes {
        match outcome {
            Ok(action) => tracing::info!(unit, ?action, "supervised"),
            Err(e) => tracing::warn!(unit, "not supervised: {e}"),
        }
    }
    eprintln!(
        "arlen-session-supervisor: {} unit(s) would be supervised and none were - \
         the systemd and broker seams are not wired, so no identity was registered",
        units.len()
    );
    std::process::ExitCode::FAILURE
}
