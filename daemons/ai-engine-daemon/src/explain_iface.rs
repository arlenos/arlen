//! The System Explanation Mode D-Bus surface. `org.arlen.AI1.explain_system`
//! runs the built-in `explain` skill on a fresh ephemeral confined pi and returns
//! its plain-language answer, re-homing Foundation §5.8 onto pi (pi-agent-adoption
//! decoupling b) so the old ai-daemon's explain path can be retired. Read-only,
//! on-demand, bounded; nothing runs in the background.

use crate::agent_iface::{resolve_dbus_caller, user_surface_admitted};
use crate::pi_run::{run_ephemeral_explain, SessionBinder};
use crate::sidecar::PiSidecar;
use arlen_ai_skills::behaviour::Behaviour;
use arlen_ai_skills::loader::{behaviour_sources, load, Provenance};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The bus name the engine owns as the drop-in replacement for the retired
/// ai-daemon (planner ruling, 8 July): pi takes over `org.arlen.AI1` rather than
/// minting a second redundant name, which also makes this the connection the
/// ai-proxy authorizes `ProxiedProvider` forwards on.
pub const EXPLAIN_BUS_NAME: &str = "org.arlen.AI1";
/// The object path the explain interface is served at.
pub const EXPLAIN_OBJECT_PATH: &str = "/org/arlen/AI1";

/// Force-load the built-in `explain` behaviour regardless of the user's `[agent]
/// enabled` list. System Explanation Mode is always available and manually
/// invoked, so it is loaded via a synthetic enable-set rather than the config.
/// Returns `None` if the skill is not present in any behaviour source.
pub fn load_explain_behaviour() -> Option<Behaviour> {
    let mut only_explain = BTreeMap::new();
    only_explain.insert("explain".to_string(), Provenance::BuiltIn);
    load(&behaviour_sources(), &only_explain)
        .loaded
        .into_iter()
        .find(|lb| lb.behaviour.manifest.name == "explain")
        .map(|lb| lb.behaviour)
}

/// Serves `explain_system` by driving a fresh ephemeral pi over the explain skill.
/// Holds the skill, the pi sidecar (the engine) and the session binder (the
/// dispatcher), all shared with the rest of the daemon.
pub struct ExplainInterface {
    behaviour: Arc<Behaviour>,
    sidecar: Arc<PiSidecar>,
    binder: Arc<dyn SessionBinder>,
}

impl ExplainInterface {
    /// Build the interface from the loaded explain skill and the daemon's shared
    /// pi sidecar and session binder.
    pub fn new(
        behaviour: Arc<Behaviour>,
        sidecar: Arc<PiSidecar>,
        binder: Arc<dyn SessionBinder>,
    ) -> Self {
        Self { behaviour, sidecar, binder }
    }
}

#[zbus::interface(name = "org.arlen.AI1")]
impl ExplainInterface {
    /// Answer "What is my computer doing right now?" (Foundation §5.8) by running
    /// the explain skill on a fresh ephemeral confined pi and returning its
    /// answer. A failure to produce one is a D-Bus error the caller surfaces.
    ///
    /// Caller-gated to the user-facing surfaces. The explain skill declares
    /// `reads: full` and its answer names the active app, the current project and
    /// recently opened files, and it is handed straight back to whoever called.
    /// The session bus is default-allow and `arlen-run` binds its socket into every
    /// confined app, so without this an app with no graph read scope could ask for
    /// a summary of the user's work and read the reply, which is the read-scope
    /// enforcement the knowledge daemon exists to apply. Unresolvable callers are
    /// refused too: fail-closed.
    ///
    /// The wire name is pinned lowercase. zbus would otherwise publish this as
    /// `ExplainSystem`, and the ai-daemon this daemon replaced carried the same
    /// `#[zbus(name)]` line for that reason - the callers, the settings app and
    /// the harness, both send `explain_system`. Re-implementing the interface
    /// without the override renamed the method out from under them: both compile,
    /// the daemon compiles, and the click answers `UnknownMethod`.
    #[zbus(name = "explain_system")]
    async fn explain_system(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        match resolve_dbus_caller(&header, connection).await {
            Ok(caller) if user_surface_admitted(&caller) => {}
            _ => return Err(zbus::fdo::Error::AccessDenied("not permitted".into())),
        }
        run_ephemeral_explain(&self.behaviour, None, &*self.sidecar, &*self.binder)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("explanation unavailable: {e}")))
    }
}
