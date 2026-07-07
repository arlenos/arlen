//! The production autonomous-curator route handler (`pi-agent-adoption.md` §E):
//! composes the two route bodies - daemon-direct DETERMINISTIC curation (auto-tag)
//! and the bounded EPHEMERAL pi run - behind the orchestrator's [`RouteHandler`]
//! seam. The orchestrator loop calls [`CuratorHandler::handle`] for each dispatched
//! behaviour; this routes it to its §E body over the daemon's real dependencies.

use crate::curation::{run_auto_tag, GraphProjectReader};
use crate::dispatch::Executor;
use crate::orchestrator::{Dispatch, Route, RouteHandler, TriggerEvent};
use crate::pi_run::{run_ephemeral_pi, SessionBinder};
use crate::sidecar::PiSidecar;
use arlen_ai_skills::loader::LoadedBehaviour;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Hard cap on CONCURRENT ephemeral pi runs. The coalescer collapses identical
/// events, but a storm of DISTINCT events (many unique paths / invite ids) each
/// gets a distinct digest and is admitted, so coalescing alone does not bound the
/// spawn rate. This bounds the in-flight confined pi processes: a run that would
/// exceed the cap is DROPPED (logged), never queued unbounded - so a trigger storm
/// cannot fork-bomb the machine with heavy LLM sessions. Small by design (an
/// autonomous curator should not be running many LLM sessions at once).
const MAX_CONCURRENT_PI_RUNS: usize = 3;

/// Routes a dispatched behaviour to its §E route body over the daemon's real
/// curation + pi-run dependencies.
pub struct CuratorHandler {
    /// Reads the projects for the auto-tag decision.
    reader: GraphProjectReader,
    /// The gated write executor the auto-tag write applies through (executor-live
    /// gated, audited, undo-registered).
    writer: Arc<dyn Executor>,
    /// The loaded behaviours, for looking up a pi-run behaviour by name.
    behaviours: Arc<Vec<LoadedBehaviour>>,
    /// The confined pi engine an ephemeral run spawns.
    sidecar: Arc<PiSidecar>,
    /// The session lifecycle for an ephemeral run (the dispatcher).
    binder: Arc<dyn SessionBinder>,
    /// Bounds concurrent ephemeral pi runs (the storm fork-bomb backstop).
    pi_run_slots: Arc<Semaphore>,
}

impl CuratorHandler {
    /// Build the handler over the daemon's real curation + pi-run dependencies.
    pub fn new(
        reader: GraphProjectReader,
        writer: Arc<dyn Executor>,
        behaviours: Arc<Vec<LoadedBehaviour>>,
        sidecar: Arc<PiSidecar>,
        binder: Arc<dyn SessionBinder>,
    ) -> Self {
        Self {
            reader,
            writer,
            behaviours,
            sidecar,
            binder,
            pi_run_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_PI_RUNS)),
        }
    }
}

impl RouteHandler for CuratorHandler {
    async fn handle(&self, event: &TriggerEvent, dispatch: &Dispatch) {
        match dispatch.route {
            Route::DeterministicCuration => {
                // Auto-tag reads on the file path the event carries; a file.opened
                // always has one, but skip fail-safe if absent (never a wrong tag).
                let Some(path) = event.fields.get("path") else {
                    tracing::debug!(behaviour = %dispatch.behaviour, "curation with no path field; skipped");
                    return;
                };
                let result = run_auto_tag(path, &self.reader, self.writer.as_ref()).await;
                tracing::info!(behaviour = %dispatch.behaviour, ?result, "deterministic curation");
            }
            Route::PiRun => {
                let Some(lb) = self
                    .behaviours
                    .iter()
                    .find(|lb| lb.behaviour.manifest.name == dispatch.behaviour)
                else {
                    tracing::warn!(behaviour = %dispatch.behaviour, "pi-run dispatched for an unknown behaviour; skipped");
                    return;
                };
                // Bound concurrent pi runs: acquire a slot or DROP this run (never
                // an unbounded queue), so a distinct-event storm cannot fork-bomb
                // the machine with confined LLM sessions. The permit is held by the
                // spawned task for the run's lifetime and released on completion.
                let Ok(permit) = self.pi_run_slots.clone().try_acquire_owned() else {
                    tracing::warn!(
                        behaviour = %dispatch.behaviour,
                        "at the concurrent pi-run cap; dropping this autonomous run"
                    );
                    return;
                };
                let behaviour = lb.behaviour.clone();
                let sidecar = self.sidecar.clone();
                let binder = self.binder.clone();
                // A pi run is bounded but can take seconds; spawn it so the
                // orchestrator loop stays responsive (the handle-returns-promptly
                // contract). The active-project anchor is a follow-up: `None` fails
                // a project-scoped read closed via GAP-21 until a Focus-Mode anchor
                // source is wired.
                tokio::spawn(async move {
                    let _permit = permit; // released when the run ends
                    let outcome =
                        run_ephemeral_pi(&behaviour, None, sidecar.as_ref(), binder.as_ref()).await;
                    tracing::info!(behaviour = %behaviour.manifest.name, ?outcome, "ephemeral pi run");
                });
            }
        }
    }
}
