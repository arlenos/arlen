// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Asking the assistant from the launcher.
//!
//! Tab flips the Waypointer into Ask mode and the pane calls `waypointer_ask`.
//! Nothing answered it until now, and the reason recorded on the missing-command
//! list was wrong in a useful direction: it said a new streamed method on
//! `org.arlen.AIAgent1` was needed. The AI engine is pi, and the daemon already
//! had the shape - a private ephemeral run that answers one question and dies -
//! written for System Explanation Mode and for the curator. What was missing was
//! an endpoint, so `org.arlen.AI1.ask` now serves one and this dials it.
//!
//! **The scope is the skill's, not this call's.** The `ask` skill declares what a
//! run may read (the current session's activity and recent work), the gate
//! enforces it per call, and the answer opens by naming the ground it stands on.
//! Passing the user's question widens nothing.
//!
//! **One turn, and that limit is real.** Each ask is its own bounded run, so a
//! follow-up does not remember the one before it. The pane sends the previous
//! session id and gets it back unchanged - honest, since nothing here fakes
//! continuity - but a conversation in this pane is a series of independent
//! answers. Carrying context would mean either threading the prior turns into the
//! question or driving the persistent pi session, which the harness owns and which
//! relays one connection at a time. That is a sequencing decision, not an
//! oversight, and it is recorded rather than papered over.

use serde::Serialize;

/// The daemon's AI surface.
const AI_BUS_NAME: &str = "org.arlen.AI1";
/// The object path it is served at.
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";
/// How long to wait for an answer. The skill's own wall-clock budget is 20s, so
/// this is that plus room for the spawn; past it the pane should say the
/// assistant is unreachable rather than hang with a spinner.
const ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// What the pane reads back: the answer, and the session it belongs to.
#[derive(Debug, Clone, Serialize)]
pub struct AskAnswer {
    /// Echoed back so the pane can keep its own thread together. See the
    /// one-turn note above: this identifies the pane's conversation, not a
    /// server-side one.
    pub session: String,
    /// The assistant's answer.
    pub text: String,
}

/// Ask the assistant a question typed into the launcher.
///
/// Every failure is an `Err` with a readable reason, because the pane turns any
/// error into "the agent isn't reachable right now" and a person deserves to find
/// the real one in the log rather than that sentence twice.
#[tauri::command]
pub async fn waypointer_ask(prompt: String, session: String) -> Result<AskAnswer, String> {
    let question = prompt.trim();
    if question.is_empty() {
        return Err("nothing was asked".to_string());
    }

    let conn = zbus::Connection::session()
        .await
        .map_err(|e| format!("no session bus: {e}"))?;
    let proxy = zbus::Proxy::new(&conn, AI_BUS_NAME, AI_OBJECT_PATH, AI_BUS_NAME)
        .await
        .map_err(|e| format!("the assistant is not on the bus: {e}"))?;

    // The method name is written out here rather than held in a const, and that is
    // deliberate: `check-dbus-method-names.py` reads call sites by matching a
    // STRING at `.call(`, so a const hides this call from the one check that
    // compares it against the interface. A name only this file can see is the
    // shape that let `explain_system` be renamed out from under two callers.
    //
    // Lowercase because the daemon pins it with `#[zbus(name = "ask")]`; zbus
    // would otherwise publish it as `Ask`.
    let text: String = tokio::time::timeout(
        ASK_TIMEOUT,
        proxy.call::<_, _, String>("ask", &(question)),
    )
    .await
    .map_err(|_| "the assistant did not answer in time".to_string())?
    .map_err(|e| format!("the assistant could not answer: {e}"))?;

    Ok(AskAnswer { session, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bus name and object path the daemon serves. The METHOD name is not
    /// here on purpose - it is a literal at the call site so
    /// `check-dbus-method-names.py` can compare it against the interface itself,
    /// which is a stronger check than this file agreeing with itself.
    #[test]
    fn the_bus_name_and_path_match_the_daemon() {
        assert_eq!(AI_BUS_NAME, "org.arlen.AI1");
        assert_eq!(AI_OBJECT_PATH, "/org/arlen/AI1");
    }

    /// An empty question never reaches the bus: it costs a confined pi spawn and
    /// a model call to be told nothing was asked.
    #[tokio::test]
    async fn an_empty_question_is_refused_before_the_bus() {
        let e = waypointer_ask("   ".to_string(), "s1".to_string())
            .await
            .expect_err("blank input is not a question");
        assert!(e.contains("nothing was asked"), "{e}");
    }
}
