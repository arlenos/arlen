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
        Self { reader, writer, behaviours, sidecar, binder }
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
                let behaviour = lb.behaviour.clone();
                let sidecar = self.sidecar.clone();
                let binder = self.binder.clone();
                // A pi run is bounded but can take seconds; spawn it so the
                // orchestrator loop stays responsive (the handle-returns-promptly
                // contract). The active-project anchor is a follow-up: `None` fails
                // a project-scoped read closed via GAP-21 until a Focus-Mode anchor
                // source is wired.
                tokio::spawn(async move {
                    let outcome =
                        run_ephemeral_pi(&behaviour, None, sidecar.as_ref(), binder.as_ref()).await;
                    tracing::info!(behaviour = %behaviour.manifest.name, ?outcome, "ephemeral pi run");
                });
            }
        }
    }
}
