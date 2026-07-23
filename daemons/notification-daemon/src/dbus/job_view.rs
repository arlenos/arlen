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

/// Convert a [`crate::job::JobView`] to the wire message the shell renders in
/// its Activity/Jobs zone. `removed = true` tells the shell to drop the job from
/// its live set (a finish/terminal prune). The enums go over the wire as their
/// stable tokens; the absent `Option`s become empty strings / zero, matching the
/// no-Option D-Bus convention the producer already speaks.
pub fn to_job_update(view: &crate::job::JobView, removed: bool) -> crate::socket::protocol::proto::JobUpdate {
    crate::socket::protocol::proto::JobUpdate {
        id: view.id,
        app_id: view.app_id.clone(),
        title: view.title.clone(),
        state: view.state.as_str().to_string(),
        state_message: view.state_message.clone().unwrap_or_default(),
        unit: view.progress.unit().as_str().to_string(),
        processed: view.progress.processed(),
        determinate: view.progress.is_determinate(),
        total: view.progress.total().unwrap_or(0),
        fraction: view.progress.fraction(),
        killable: view.capabilities.killable,
        suspendable: view.capabilities.suspendable,
        started_at: view.started_at,
        egress_host: view.egress_host.clone().unwrap_or_default(),
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::to_job_update;
    use crate::job::{JobRegistry, JobViewServer};

    #[test]
    fn to_job_update_maps_the_whole_view() {
        let s = JobViewServer::new(std::sync::Arc::new(std::sync::Mutex::new(JobRegistry::new())));
        let id = s.register(
            "files".into(),
            "Copy 200 files".into(),
            "files",
            Some(200),
            true,
            true,
            Some("api.example.com".into()),
            42,
        );
        s.update(id, 50, None);
        let view = s.snapshot().into_iter().next().unwrap();

        let msg = to_job_update(&view, false);
        assert_eq!(msg.id, id);
        assert_eq!(msg.app_id, "files");
        assert_eq!(msg.title, "Copy 200 files");
        assert_eq!(msg.state, "running");
        assert_eq!(msg.unit, "files");
        assert_eq!(msg.processed, 50);
        assert!(msg.determinate);
        assert_eq!(msg.total, 200);
        assert!((msg.fraction - 0.25).abs() < 1e-9);
        assert!(msg.killable && msg.suspendable);
        assert_eq!(msg.started_at, 42);
        assert_eq!(msg.egress_host, "api.example.com");
        assert!(!msg.removed);

        // The prune form flips only the removed flag.
        assert!(to_job_update(&view, true).removed);
    }
}
