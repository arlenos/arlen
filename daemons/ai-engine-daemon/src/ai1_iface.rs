//! The `org.arlen.AI1` D-Bus surface: the two questions a user-facing surface may
//! put to the assistant, each answered by a fresh ephemeral confined pi.
//!
//! `explain_system` runs the built-in `explain` skill and answers "what is my
//! computer doing right now", re-homing Foundation §5.8 onto pi (pi-agent-adoption
//! decoupling b) so the old ai-daemon's explain path can be retired.
//!
//! `ask` runs the built-in `ask` skill against a question the user typed, for the
//! launcher's assistant pane. Same shape, different skill and a caller-supplied
//! turn - and the skill is what decides what the run may read, so the question
//! widens nothing.
//!
//! Both are read-only, on-demand and bounded; nothing here runs in the background.

use crate::agent_iface::{resolve_dbus_caller, user_surface_admitted};
use crate::pi_run::{run_ephemeral_answer, run_ephemeral_explain, SessionBinder};
use crate::sidecar::PiSidecar;
use arlen_ai_skills::behaviour::Behaviour;
use arlen_ai_skills::loader::{behaviour_sources, load, Provenance};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The bus name the engine owns as the drop-in replacement for the retired
/// ai-daemon (planner ruling, 8 July): pi takes over `org.arlen.AI1` rather than
/// minting a second redundant name, which also makes this the connection the
/// ai-proxy authorizes `ProxiedProvider` forwards on.
pub const AI1_BUS_NAME: &str = "org.arlen.AI1";
/// The object path the interface is served at.
pub const AI1_OBJECT_PATH: &str = "/org/arlen/AI1";

/// Force-load a built-in behaviour by name, regardless of the user's `[agent]
/// enabled` list. The skills served here are always available and manually
/// invoked - a user asking a question is the trigger - so they load via a
/// synthetic enable-set rather than the config, which governs what runs on its
/// own. Returns `None` if the skill is not present in any behaviour source.
pub fn load_builtin_behaviour(name: &str) -> Option<Behaviour> {
    let mut only = BTreeMap::new();
    only.insert(name.to_string(), Provenance::BuiltIn);
    load(&behaviour_sources(), &only)
        .loaded
        .into_iter()
        .find(|lb| lb.behaviour.manifest.name == name)
        .map(|lb| lb.behaviour)
}

/// Serves the `org.arlen.AI1` methods by driving a fresh ephemeral pi over the
/// skill each one names. Holds those skills, the pi sidecar (the engine) and the
/// session binder (the dispatcher), all shared with the rest of the daemon.
///
/// `ask` is optional where `explain` is not: a missing `ask` skill leaves the
/// launcher pane without an answer, which is a degraded surface, while the daemon
/// still serves System Explanation Mode. Failing both because one skill file is
/// absent would be the wrong trade.
pub struct Ai1Interface {
    behaviour: Arc<Behaviour>,
    ask: Option<Arc<Behaviour>>,
    sidecar: Arc<PiSidecar>,
    binder: Arc<dyn SessionBinder>,
}

impl Ai1Interface {
    /// Build the interface from the loaded skills and the daemon's shared pi
    /// sidecar and session binder.
    pub fn new(
        behaviour: Arc<Behaviour>,
        ask: Option<Arc<Behaviour>>,
        sidecar: Arc<PiSidecar>,
        binder: Arc<dyn SessionBinder>,
    ) -> Self {
        Self { behaviour, ask, sidecar, binder }
    }
}

#[zbus::interface(name = "org.arlen.AI1")]
impl Ai1Interface {
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

    /// Answer a question the user typed, by running the `ask` skill on a fresh
    /// ephemeral confined pi.
    ///
    /// Caller-gated exactly like `explain_system`, and for the same reason: the
    /// answer is drawn from the user's own graph and handed straight back, so an
    /// app with no read scope must not be able to get one by asking nicely. The
    /// session bus is default-allow and `arlen-run` binds it into every confined
    /// app, so without this gate the read-scope enforcement the knowledge daemon
    /// exists to apply would have a way around it.
    ///
    /// **The question does not widen anything.** What the run may read is declared
    /// by the `ask` skill, and the gate enforces that per call whatever is asked -
    /// the turn is the question, the skill is the authority.
    ///
    /// An empty question is refused here rather than sent: it costs a confined pi
    /// spawn and a model call to be told nothing was asked.
    #[zbus(name = "ask")]
    async fn ask(
        &self,
        question: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        match resolve_dbus_caller(&header, connection).await {
            Ok(caller) if user_surface_admitted(&caller) => {}
            _ => return Err(zbus::fdo::Error::AccessDenied("not permitted".into())),
        }
        let question = question.trim();
        if question.is_empty() {
            return Err(zbus::fdo::Error::InvalidArgs("no question was asked".into()));
        }
        let Some(skill) = self.ask.as_ref() else {
            return Err(zbus::fdo::Error::Failed(
                "the ask skill is not installed, so there is nothing to answer with".into(),
            ));
        };
        run_ephemeral_answer(skill, None, question, &*self.sidecar, &*self.binder)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))
    }
}
