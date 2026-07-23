//! The `org.arlen.JobViewServer1` D-Bus interface.
//!
//! Producers (file-manager, forage, model-manager) register and update
//! long-running jobs here; the shell reads them over the daemon's existing
//! socket broadcast (job-progress-surface.md's converged model - one daemon, two
//! object types, no second socket). This is a thin adapter over the tested
//! [`crate::job::JobViewServer`] logic: it only maps D-Bus-friendly types (D-Bus
//! has no `Option`, so a `determinate` flag stands in for a known total and an
//! empty string stands in for an absent host/message) onto the model.

use zbus::interface;

use crate::job::JobViewServer;

/// The object served at `/org/arlen/JobViewServer` on the session bus.
pub struct JobViewDbus {
    server: JobViewServer,
}

impl JobViewDbus {
    /// Build over the shared job server.
    pub fn new(server: JobViewServer) -> Self {
        JobViewDbus { server }
    }
}

#[interface(name = "org.arlen.JobViewServer1")]
impl JobViewDbus {
    /// Register a long-running job and return its stable id. `determinate=false`
    /// starts it indeterminate (the `total` is ignored until a later `update`
    /// supplies one). `unit` is one of `bytes`/`files`/`directories`/`items`
    /// (an unknown unit renders as a generic item count). An empty `egress_host`
    /// means the job does not reach the network; a non-empty one is surfaced at
    /// the consent moment.
    #[allow(clippy::too_many_arguments)]
    async fn register(
        &self,
        app_id: String,
        title: String,
        unit: String,
        total: u64,
        determinate: bool,
        killable: bool,
        suspendable: bool,
        egress_host: String,
    ) -> u64 {
        let total = determinate.then_some(total);
        let host = (!egress_host.is_empty()).then_some(egress_host);
        self.server.register(
            app_id,
            title,
            &unit,
            total,
            killable,
            suspendable,
            host,
            now_micros(),
        )
    }

    /// Advance a job's amounts in its real unit. `determinate=false` leaves the
    /// total unknown (an indeterminate stretch). Returns `false` for an unknown
    /// id - a late update after the job finished is a harmless no-op.
    async fn update(&self, id: u64, processed: u64, total: u64, determinate: bool) -> bool {
        self.server.update(id, processed, determinate.then_some(total))
    }

    /// Set a job's lifecycle state and its explanatory message. `state` is one
    /// of `running`/`paused`/`impeded`/`error-recoverable`/`error-fatal`/`done`;
    /// an unknown token is rejected (returns `false`). Pass an empty `message`
    /// for `running`, an explanation for every other state.
    async fn set_state(&self, id: u64, state: String, message: String) -> bool {
        let message = (!message.is_empty()).then_some(message);
        self.server.set_state(id, &state, message)
    }

    /// Remove a finished job from the live set. Returns whether it existed.
    async fn finish(&self, id: u64) -> bool {
        self.server.remove(id)
    }
}

/// The current wall-clock in epoch micros, for a job's start timestamp.
fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
