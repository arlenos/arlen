//! The client half of an LSP session: the handshake, document sync, and what
//! arrives back.
//!
//! A state machine over [`crate::wire`], with no IO in it. The editor host owns
//! the process and feeds this the bytes; this decides what to send and what the
//! surface should be told. That split is what makes the protocol testable
//! without a language server, and it is also where the interesting rules live -
//! most of what goes wrong with an LSP client is ORDER, not parsing.

use std::collections::HashMap;

use serde_json::json;

use crate::wire::{Incoming, Outgoing};

/// Where the session is in its handshake.
///
/// The order is not decoration. A server may reject, ignore or crash on document
/// notifications that arrive before `initialized`, and the failure looks like
/// "the language server does not work" rather than "we spoke too early".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// `initialize` sent, waiting for its reply.
    Handshaking,
    /// The server answered and `initialized` went out. Documents may flow.
    Ready,
    /// The server refused to start, or answered the handshake with an error.
    Failed,
    /// `shutdown` sent; nothing further may be sent on this session.
    ShuttingDown,
}

/// Something the editor surface should act on.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The handshake completed. Language features are available from here.
    Ready,
    /// Diagnostics for one document, replacing whatever was shown for it. LSP
    /// publishes the WHOLE set per document, so an empty list means "this file
    /// is clean now" and must clear the old ones rather than being ignored.
    Diagnostics { uri: String, items: Vec<Diagnostic> },
    /// The server said something went wrong, in its own words.
    ServerError(String),
    /// A message this client does not handle. Kept rather than dropped so the
    /// host can log it: silence about an unknown method is how a missing feature
    /// becomes a mystery.
    Unhandled(String),
}

/// One diagnostic, reduced to what the editor renders.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Zero-based line, as LSP counts them.
    pub line: u32,
    pub character: u32,
    /// 1 error, 2 warning, 3 information, 4 hint. Absent in the protocol means
    /// the server left it to the client, which this reports as information
    /// rather than inventing an error.
    pub severity: u8,
    pub message: String,
}

/// What went wrong with a request the caller made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// A document notification before the handshake finished.
    NotReady(Phase),
    /// A change to a document that was never opened. The server has no text to
    /// apply it to, and sending it anyway desynchronises its copy from ours.
    NotOpen(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady(p) => write!(f, "the session is {p:?}, so documents cannot be sent yet"),
            Self::NotOpen(uri) => write!(f, "{uri} was never opened, so it cannot be changed"),
        }
    }
}

impl std::error::Error for SessionError {}

/// The client side of one language server connection.
pub struct Session {
    phase: Phase,
    next_id: i64,
    /// Open documents and the version last sent for each. LSP requires the
    /// version to increase; repeating one leaves servers free to ignore the
    /// change, which shows up as diagnostics that are silently one edit stale.
    open: HashMap<String, i64>,
}

impl Session {
    /// Begin a session, returning the `initialize` request to send.
    ///
    /// `root` is the project directory the server will index. The capabilities
    /// declared are only those this client actually implements - claiming one it
    /// does not honour makes the server send messages that are then dropped.
    pub fn start(root_uri: &str) -> (Self, Outgoing) {
        let mut s = Self {
            phase: Phase::Handshaking,
            next_id: 1,
            open: HashMap::new(),
        };
        let id = s.take_id();
        let req = Outgoing::request(
            id,
            "initialize",
            json!({
                "processId": null,
                "rootUri": root_uri,
                "capabilities": {
                    "textDocument": {
                        "synchronization": { "didSave": false, "willSave": false },
                        "publishDiagnostics": {},
                    }
                },
            }),
        );
        (s, req)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    fn take_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Feed one message from the server. Returns what to send back and what the
    /// surface should be told.
    pub fn receive(&mut self, msg: &Incoming) -> (Vec<Outgoing>, Vec<Event>) {
        // An error reply during the handshake is terminal: there is no session.
        if self.phase == Phase::Handshaking && msg.id.is_some() {
            if let Some(err) = &msg.error {
                self.phase = Phase::Failed;
                let text = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("the server refused to initialize");
                return (vec![], vec![Event::ServerError(text.to_string())]);
            }
            if msg.result.is_some() {
                self.phase = Phase::Ready;
                return (
                    vec![Outgoing::notification("initialized", json!({}))],
                    vec![Event::Ready],
                );
            }
        }

        match msg.method.as_deref() {
            Some("textDocument/publishDiagnostics") => {
                let p = msg.params.clone().unwrap_or(json!({}));
                let uri = p.get("uri").and_then(|u| u.as_str()).unwrap_or_default();
                (vec![], vec![Event::Diagnostics { uri: uri.to_string(), items: parse_diagnostics(&p) }])
            }
            // Server-to-client requests this client does not implement still need
            // an answer, or the server waits forever. `window/showMessageRequest`
            // is the common one; a null result means "no action taken", which is
            // true and lets it continue.
            Some(other) if msg.id.is_some() => {
                let id = msg.id.clone().and_then(|v| v.as_i64()).unwrap_or(0);
                (
                    vec![Outgoing {
                        jsonrpc: "2.0",
                        id: Some(id),
                        method: String::new(),
                        params: None,
                    }],
                    vec![Event::Unhandled(other.to_string())],
                )
            }
            Some(other) => (vec![], vec![Event::Unhandled(other.to_string())]),
            None => (vec![], vec![]),
        }
    }

    /// Tell the server a document is open, with its full text.
    pub fn did_open(
        &mut self,
        uri: &str,
        language_id: &str,
        text: &str,
    ) -> Result<Outgoing, SessionError> {
        if self.phase != Phase::Ready {
            return Err(SessionError::NotReady(self.phase));
        }
        self.open.insert(uri.to_string(), 1);
        Ok(Outgoing::notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        ))
    }

    /// Send the document's new full text.
    ///
    /// Full-text sync, deliberately: incremental sync is a range calculation this
    /// client would have to get exactly right on every edit, and one wrong range
    /// leaves the server's copy silently different from the buffer for the rest
    /// of the session. Full text is more bytes and cannot drift.
    pub fn did_change(&mut self, uri: &str, text: &str) -> Result<Outgoing, SessionError> {
        if self.phase != Phase::Ready {
            return Err(SessionError::NotReady(self.phase));
        }
        let version = self
            .open
            .get_mut(uri)
            .ok_or_else(|| SessionError::NotOpen(uri.to_string()))?;
        *version += 1;
        Ok(Outgoing::notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": *version },
                "contentChanges": [ { "text": text } ],
            }),
        ))
    }

    /// Tell the server a document is closed. Its diagnostics stop being ours to
    /// show.
    pub fn did_close(&mut self, uri: &str) -> Result<Outgoing, SessionError> {
        if self.phase != Phase::Ready {
            return Err(SessionError::NotReady(self.phase));
        }
        self.open.remove(uri);
        Ok(Outgoing::notification(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        ))
    }

    /// Ask the server to stop. After this nothing else may be sent.
    pub fn shutdown(&mut self) -> Outgoing {
        let id = self.take_id();
        self.phase = Phase::ShuttingDown;
        Outgoing::request(id, "shutdown", json!(null))
    }
}

fn parse_diagnostics(params: &serde_json::Value) -> Vec<Diagnostic> {
    params
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|d| {
                    let start = d.get("range")?.get("start")?;
                    Some(Diagnostic {
                        line: start.get("line")?.as_u64()? as u32,
                        character: start.get("character")?.as_u64()? as u32,
                        // Absent severity is the server leaving it to us. Report
                        // information rather than inventing an error: a warning
                        // rendered as an error is a lie about the code.
                        severity: d.get("severity").and_then(|s| s.as_u64()).unwrap_or(3) as u8,
                        message: d.get("message")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming(v: serde_json::Value) -> Incoming {
        serde_json::from_value(v).expect("valid message")
    }

    fn ready() -> Session {
        let (mut s, _) = Session::start("file:///work");
        let (out, ev) = s.receive(&incoming(json!({ "id": 1, "result": { "capabilities": {} } })));
        assert_eq!(ev, vec![Event::Ready]);
        assert_eq!(out.len(), 1, "the handshake is not done until `initialized` goes out");
        s
    }

    #[test]
    fn the_handshake_answers_with_initialized() {
        let (s, req) = Session::start("file:///work");
        assert_eq!(s.phase(), Phase::Handshaking);
        let json: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["method"], "initialize");
        assert_eq!(json["params"]["rootUri"], "file:///work");
        let s = ready();
        assert_eq!(s.phase(), Phase::Ready);
    }

    /// The order rule, and the reason it is worth a test: a server may reject,
    /// ignore or crash on a document notification sent before `initialized`, and
    /// the symptom is "the language server does not work".
    #[test]
    fn documents_cannot_be_sent_before_the_handshake_finishes() {
        let (mut s, _) = Session::start("file:///work");
        assert_eq!(
            s.did_open("file:///work/a.rs", "rust", "fn main() {}").unwrap_err(),
            SessionError::NotReady(Phase::Handshaking)
        );
    }

    #[test]
    fn a_refused_handshake_is_terminal_and_says_why() {
        let (mut s, _) = Session::start("file:///work");
        let (out, ev) = s.receive(&incoming(
            json!({ "id": 1, "error": { "code": -32603, "message": "no toolchain" } }),
        ));
        assert!(out.is_empty(), "nothing more is sent to a server that refused");
        assert_eq!(ev, vec![Event::ServerError("no toolchain".into())]);
        assert_eq!(s.phase(), Phase::Failed);
    }

    /// Versions must increase or a server is free to ignore the change, which
    /// shows up as diagnostics that are quietly one edit behind the buffer.
    #[test]
    fn each_change_carries_a_higher_version_than_the_last() {
        let mut s = ready();
        s.did_open("file:///a.rs", "rust", "one").unwrap();
        let v = |o: &Outgoing| serde_json::to_value(o).unwrap()["params"]["textDocument"]["version"].as_i64().unwrap();
        let first = s.did_change("file:///a.rs", "two").unwrap();
        let second = s.did_change("file:///a.rs", "three").unwrap();
        assert_eq!(v(&first), 2);
        assert_eq!(v(&second), 3);
    }

    #[test]
    fn a_change_to_an_unopened_document_is_refused() {
        let mut s = ready();
        assert_eq!(
            s.did_change("file:///never.rs", "x").unwrap_err(),
            SessionError::NotOpen("file:///never.rs".into())
        );
        // And a closed document is unopened again, so the same refusal holds.
        s.did_open("file:///a.rs", "rust", "one").unwrap();
        s.did_close("file:///a.rs").unwrap();
        assert!(matches!(s.did_change("file:///a.rs", "x"), Err(SessionError::NotOpen(_))));
    }

    #[test]
    fn diagnostics_are_read_out_with_their_positions() {
        let mut s = ready();
        let (_, ev) = s.receive(&incoming(json!({
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///a.rs",
                "diagnostics": [
                    { "range": { "start": { "line": 3, "character": 8 } },
                      "severity": 1, "message": "cannot find value `x`" }
                ]
            }
        })));
        assert_eq!(
            ev,
            vec![Event::Diagnostics {
                uri: "file:///a.rs".into(),
                items: vec![Diagnostic {
                    line: 3,
                    character: 8,
                    severity: 1,
                    message: "cannot find value `x`".into()
                }]
            }]
        );
    }

    /// An empty list is a statement: this file is clean now. Treating it as
    /// nothing-to-do leaves the previous errors on screen after they are fixed.
    #[test]
    fn an_empty_diagnostic_list_is_still_an_answer() {
        let mut s = ready();
        let (_, ev) = s.receive(&incoming(json!({
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": "file:///a.rs", "diagnostics": [] }
        })));
        assert_eq!(ev, vec![Event::Diagnostics { uri: "file:///a.rs".into(), items: vec![] }]);
    }

    #[test]
    fn a_diagnostic_without_a_severity_is_not_promoted_to_an_error() {
        let mut s = ready();
        let (_, ev) = s.receive(&incoming(json!({
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": "file:///a.rs", "diagnostics": [
                { "range": { "start": { "line": 0, "character": 0 } }, "message": "consider renaming" }
            ] }
        })));
        let Event::Diagnostics { items, .. } = &ev[0] else { panic!("diagnostics") };
        assert_eq!(items[0].severity, 3, "absent severity is information, not an error");
    }

    /// A server-to-client REQUEST that goes unanswered leaves the server
    /// waiting. Answering with a null result says "no action taken", which is
    /// true and lets it get on with indexing.
    #[test]
    fn an_unhandled_server_request_still_gets_a_reply() {
        let mut s = ready();
        let (out, ev) = s.receive(&incoming(json!({
            "id": 7, "method": "window/showMessageRequest", "params": {}
        })));
        assert_eq!(out.len(), 1, "a request must be answered or the server waits");
        assert_eq!(serde_json::to_value(&out[0]).unwrap()["id"], 7);
        assert_eq!(ev, vec![Event::Unhandled("window/showMessageRequest".into())]);

        // A NOTIFICATION with no id gets no reply, because none is expected.
        let (out, _) = s.receive(&incoming(json!({ "method": "window/logMessage", "params": {} })));
        assert!(out.is_empty(), "answering a notification is a protocol error of our own");
    }
}
