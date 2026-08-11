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

/// The engine this interface answers with, or the reason there is none.
///
/// It carries the reason rather than being an `Option` because the reason is the
/// whole point: a machine with no pi runtime and a machine with AI switched off
/// are different answers to "why did nothing happen", and the caller is a person
/// who has just typed a question and is owed the true one.
pub enum Engine {
    /// A sidecar is configured and a run can be spawned.
    Ready(Arc<PiSidecar>),
    /// No run is possible, and this is what to tell whoever asks.
    Unavailable(Arc<str>),
}

impl Engine {
    /// The sidecar, or a D-Bus error carrying the reason there is none.
    ///
    /// **The interface is served whether or not an engine exists, and this method
    /// is why that is safe.** Registration used to sit inside the arm where the
    /// sidecar paths resolved, while the bus NAME was claimed unconditionally - so
    /// on a machine without the pi runtime the daemon owned `org.arlen.AI1` and
    /// served nothing at `/org/arlen/AI1`, and the launcher's Ask pane came back
    /// with `Unknown object`. A name without an object is an absence, and an
    /// absence has no reason, no subject and no line in the ledger. Now the object
    /// is always there and says why it cannot answer.
    fn ready(&self) -> zbus::fdo::Result<&PiSidecar> {
        match self {
            Engine::Ready(sidecar) => Ok(sidecar),
            Engine::Unavailable(why) => Err(zbus::fdo::Error::Failed(why.to_string())),
        }
    }
}

/// Serves the `org.arlen.AI1` methods by driving a fresh ephemeral pi over the
/// skill each one names. Holds those skills, the engine and the session binder
/// (the dispatcher), all shared with the rest of the daemon.
///
/// **Every piece is optional and the object is served regardless.** A missing
/// skill, a missing engine, an assistant switched off: each is a method that
/// refuses with its own true reason, not a method that is not there. This was
/// learned the hard way twice in one evening - the object hung off `ai_enabled`
/// and off the sidecar resolving, both were removed, and it was STILL absent
/// because the explain skill had not been found either. Three conditions, one
/// symptom, and only a running daemon ever showed it.
pub struct Ai1Interface {
    behaviour: Option<Arc<Behaviour>>,
    ask: Option<Arc<Behaviour>>,
    engine: Engine,
    binder: Arc<dyn SessionBinder>,
}

impl Ai1Interface {
    /// Build the interface from the loaded skills, the daemon's session binder,
    /// and either a pi sidecar or the reason there is not one.
    pub fn new(
        behaviour: Option<Arc<Behaviour>>,
        ask: Option<Arc<Behaviour>>,
        engine: Engine,
        binder: Arc<dyn SessionBinder>,
    ) -> Self {
        Self { behaviour, ask, engine, binder }
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
        let Some(skill) = self.behaviour.as_ref() else {
            return Err(zbus::fdo::Error::Failed(
                "the explain skill is not installed, so there is nothing to explain with".into(),
            ));
        };
        let engine = self.engine.ready()?;
        run_ephemeral_explain(skill, None, engine, &*self.binder)
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
        let engine = self.engine.ready()?;
        run_ephemeral_answer(skill, None, question, engine, &*self.binder)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this pairs with: the object was registered only where a sidecar
    /// resolved, so a machine without the pi runtime owned the name and served
    /// nothing at its path. Now the object exists and the caller is told why it
    /// cannot answer, which is the difference between a refusal and an absence.
    #[test]
    fn an_engineless_interface_refuses_with_the_reason() {
        let e = match Engine::Unavailable("the assistant is switched off".into()).ready() {
            Ok(_) => panic!("an unavailable engine must not hand back a sidecar"),
            Err(e) => e,
        };
        assert!(e.to_string().contains("switched off"), "the refusal must carry the reason: {e}");
    }

    /// And the reason is whatever the daemon passed, not a fixed string. The two
    /// causes it distinguishes - switched off, and not set up here - are different
    /// answers to the same question, and a person can act on only one of them.
    #[test]
    fn the_refusal_carries_the_daemons_own_reason() {
        let e = match Engine::Unavailable("the assistant is not set up on this machine".into())
            .ready()
        {
            Ok(_) => panic!("an unavailable engine must not hand back a sidecar"),
            Err(e) => e,
        };
        assert!(e.to_string().contains("not set up on this machine"), "{e}");
    }
}
