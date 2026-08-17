// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Record a completion session and replay it, so a change to the run loop has a
//! deterministic oracle instead of a live model.
//!
//! The echo provider already answers without a model, which makes the chain
//! runnable offline - but it answers the SAME sentence to everything, so it can
//! only prove that the plumbing carries a response, never that a change to the
//! loop still produces the same conversation. A recorded session closes that: run
//! once against a real model, keep the exchange, and every later run of the same
//! script must reproduce it turn for turn.
//!
//! The tape is JSON Lines, one `{"request": ..., "response": ...}` per completion,
//! in the order they happened. Line-oriented on purpose: a session that crashes
//! mid-run leaves every completed turn readable, which is the state you most want
//! a recording from.
//!
//! REQUESTS ARE MATCHED, NOT COUNTED. Replaying strictly by position would answer
//! a changed request with the old response and call the run a pass - the exact
//! shape of a test that cannot fail. The key is the request itself, so a loop that
//! now asks something different gets [`ReplayError::Unexpected`] and the run stops
//! being evidence about anything.
//!
//! What this is NOT: a mock of the provider's behaviour. It reproduces one
//! recorded session. A new prompt has no answer on the tape and must be recorded
//! again against a real model.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One recorded exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// The completion request body, verbatim.
    pub request: String,
    /// The response body the provider gave for it, verbatim.
    pub response: String,
}

/// Why a replay could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// The tape holds no response for this request. The loop asked something the
    /// recorded session never asked.
    Unexpected {
        /// A bounded excerpt, so the message is readable in a log without
        /// carrying a whole prompt into it.
        excerpt: String,
    },
    /// The tape itself could not be read or parsed.
    Tape(String),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unexpected { excerpt } => write!(
                f,
                "no recorded response for this request: {excerpt}. The loop asked \
                 something the recorded session did not; re-record it."
            ),
            Self::Tape(e) => write!(f, "the recording could not be read: {e}"),
        }
    }
}

/// The identity of a request for lookup: its bytes, whitespace-insensitive.
///
/// Not a hash of the parsed JSON, because key order is stable within one
/// serializer and comparing the text keeps the tape greppable. Whitespace is
/// normalised so a pretty-printed body and a compact one are the same turn.
fn key_of(request: &str) -> String {
    request.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Appends turns to a tape file as they happen.
pub struct Recorder {
    path: PathBuf,
}

impl Recorder {
    /// Start (or continue) a recording at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Append one exchange. Written and flushed per turn, so a run that dies
    /// still leaves everything up to the last completed turn on disk.
    pub fn record(&self, turn: &Turn) -> Result<(), String> {
        let line = serde_json::json!({
            "request": turn.request,
            "response": turn.response,
        })
        .to_string();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{e}"))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("{e}"))?;
        writeln!(f, "{line}").map_err(|e| format!("{e}"))?;
        f.flush().map_err(|e| format!("{e}"))
    }
}

/// Answers completions from a recorded tape.
#[derive(Debug)]
pub struct Replayer {
    by_request: HashMap<String, String>,
    /// Turn count, so a caller can report what it is replaying against.
    turns: usize,
}

impl Replayer {
    /// Load a tape. A malformed line is an error rather than a skipped turn: a
    /// replay that silently drops a turn answers a later request with the wrong
    /// response, which is worse than refusing to start.
    pub fn load(path: &Path) -> Result<Self, ReplayError> {
        let text = std::fs::read_to_string(path).map_err(|e| ReplayError::Tape(format!("{e}")))?;
        let mut by_request = HashMap::new();
        let mut turns = 0;
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| ReplayError::Tape(format!("line {}: {e}", i + 1)))?;
            let (Some(req), Some(resp)) = (v["request"].as_str(), v["response"].as_str()) else {
                return Err(ReplayError::Tape(format!(
                    "line {}: expected string `request` and `response`",
                    i + 1
                )));
            };
            by_request.insert(key_of(req), resp.to_string());
            turns += 1;
        }
        Ok(Self { by_request, turns })
    }

    /// How many turns the tape holds.
    pub fn turns(&self) -> usize {
        self.turns
    }

    /// The recorded response for `request`, or why there is none.
    pub fn answer(&self, request: &str) -> Result<&str, ReplayError> {
        self.by_request
            .get(&key_of(request))
            .map(String::as_str)
            .ok_or_else(|| ReplayError::Unexpected {
                excerpt: request.chars().take(120).collect(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("arlen-replay-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).expect("dir");
        d
    }

    #[test]
    fn a_recorded_turn_replays_verbatim() {
        let dir = tape_dir("verbatim");
        let tape = dir.join("session.jsonl");
        let rec = Recorder::new(&tape);
        rec.record(&Turn {
            request: r#"{"messages":[{"role":"user","content":"hi"}]}"#.into(),
            response: r#"{"choices":[{"message":{"content":"hello"}}]}"#.into(),
        })
        .expect("record");

        let r = Replayer::load(&tape).expect("load");
        assert_eq!(r.turns(), 1);
        assert_eq!(
            r.answer(r#"{"messages":[{"role":"user","content":"hi"}]}"#).unwrap(),
            r#"{"choices":[{"message":{"content":"hello"}}]}"#
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The property the whole thing exists for: a loop that now asks something
    /// else must not be handed the old answer.
    #[test]
    fn a_request_the_session_never_made_is_refused() {
        let dir = tape_dir("unexpected");
        let tape = dir.join("session.jsonl");
        Recorder::new(&tape)
            .record(&Turn { request: "ask A".into(), response: "answer A".into() })
            .expect("record");

        let r = Replayer::load(&tape).expect("load");
        match r.answer("ask B") {
            Err(ReplayError::Unexpected { excerpt }) => assert!(excerpt.contains("ask B")),
            other => panic!("expected a refusal, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Formatting is not a different question.
    #[test]
    fn whitespace_does_not_make_a_new_turn() {
        let dir = tape_dir("whitespace");
        let tape = dir.join("session.jsonl");
        Recorder::new(&tape)
            .record(&Turn { request: "{\"a\": 1}".into(), response: "ok".into() })
            .expect("record");

        let r = Replayer::load(&tape).expect("load");
        assert_eq!(r.answer("{\"a\":\n   1}").unwrap(), "ok");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A half-written tape is a broken oracle, and a broken oracle that answers
    /// is worse than one that refuses to load.
    #[test]
    fn a_malformed_line_refuses_to_load() {
        let dir = tape_dir("malformed");
        let tape = dir.join("session.jsonl");
        std::fs::write(&tape, "{\"request\":\"a\",\"response\":\"b\"}\nnot json\n").expect("write");
        match Replayer::load(&tape) {
            Err(ReplayError::Tape(m)) => assert!(m.contains("line 2")),
            other => panic!("expected a tape error, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_completed_turn_survives_a_run_that_stops_early() {
        let dir = tape_dir("partial");
        let tape = dir.join("session.jsonl");
        let rec = Recorder::new(&tape);
        rec.record(&Turn { request: "one".into(), response: "1".into() }).expect("record");
        rec.record(&Turn { request: "two".into(), response: "2".into() }).expect("record");
        // No close, no finalise: the process simply stops here.
        let r = Replayer::load(&tape).expect("load");
        assert_eq!(r.turns(), 2);
        assert_eq!(r.answer("two").unwrap(), "2");
        std::fs::remove_dir_all(&dir).ok();
    }
}
