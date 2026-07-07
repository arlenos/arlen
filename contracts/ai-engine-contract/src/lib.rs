//! The engine-neutral Arlen AI contract (`pi-agent-adoption.md` §A).
//!
//! Five Arlen verbs express everything the AI frame needs from a loop engine, so
//! the engine (pi today, a different one tomorrow) is swappable behind a stable
//! daemon contract: SessionInit, Authorize, Execute, Report, Confirm. The
//! distinctive features (gate, audit, compensation, KG scoping, screening) live
//! in the daemon FRAME, never in the engine.
//!
//! These types are deliberately ENGINE-NEUTRAL: tool inputs/outputs are opaque
//! JSON (`serde_json::Value`), and nothing here couples to `ai-core` or to pi.
//! The daemon maps an `Authorize` to `Capability::decide`, a `Report` to the
//! audit/compensation/screening path, etc.; this crate is only the wire
//! vocabulary the daemon and the engine's thin plugins exchange over the
//! daemon's Unix socket.
//!
//! Wire stability matters: the daemon and a (possibly older) engine plugin must
//! agree on the tags, so the serde `tag` names are part of the contract and are
//! locked by tests, the same discipline `modulesd-proto` follows.

use serde::{Deserialize, Serialize};

/// How much of the Knowledge Graph a session may read. Engine-neutral mirror of
/// the daemon's read-scope tier (the daemon resolves the concrete bounded
/// subgraph; the engine only learns the coarse level for prompt context).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadTier {
    /// No graph read at all.
    None,
    /// The narrowest read scope (current session only).
    Minimal,
    /// A standard working scope.
    Standard,
    /// A wider scope (e.g. the active project plus recent context).
    Extended,
    /// The broadest scope a session can be granted.
    Full,
}

/// The coarse authority a session carries, for prompt context only. The daemon
/// enforces the real grant server-side per call (the engine is never trusted
/// with it); this is what the engine may *tell the model* it can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityContext {
    /// The generic agent tools (bash, fs-in-workspace, etc.) the session may use
    /// in-engine, each still gated per call via [`Authorize`]. Empty = none.
    pub generic_tools: Vec<String>,
    /// The privileged proxy tools (KG read/write, OS ops) the session may *ask*
    /// the daemon to run via [`Execute`]. The daemon runs them in trusted Rust.
    pub proxy_tools: Vec<String>,
}

/// Daemon to engine: initialise a session. Maps to pi `before_agent_start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInit {
    /// The system prompt the daemon composed for this session.
    pub system_prompt: String,
    /// The active behaviour/skill id driving the session, if any.
    pub behaviour: Option<String>,
    /// The session's coarse authority (prompt context only).
    pub capability_context: CapabilityContext,
    /// The active-project anchor that bounds graph reads (the GAP-21 fix).
    /// `None` means no project is anchored for this session.
    pub project_anchor: Option<String>,
    /// How much of the graph this session may read.
    pub read_tier: ReadTier,
    /// Whether the whole run was started by external content (HIGH-2). Set by the
    /// SUPERVISOR from the session's origin, never by the engine; the daemon ORs it
    /// with each call's own `external_triggered` (escalate-only) so an externally-
    /// originated session escalates every action to a confirmation. Additive on the
    /// wire (defaults false) so an older engine's SessionInit still deserializes.
    #[serde(default)]
    pub externally_triggered: bool,
}

/// Engine to daemon: a tool the model wants to call, for authorization. Maps to
/// pi `tool_call`. `external_triggered` is true when the run was started by an
/// external event (not a direct user request), which always escalates the
/// decision (prompt-injection containment).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authorize {
    /// The tool name as the engine sees it.
    pub tool_name: String,
    /// The proposed tool arguments (opaque to this contract).
    pub tool_input: serde_json::Value,
    /// Whether this run was triggered by external content.
    pub external_triggered: bool,
}

/// The daemon's verdict on an [`Authorize`]. The result of `Capability::decide`,
/// projected engine-neutrally. The engine enforces it inline: `Deny` blocks the
/// call, `Modify` replaces the args, `Confirm` waits for the consent answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum AuthorizeDecision {
    /// Run the tool as proposed. `proof` is the one-time execution proof the engine
    /// must present at [`Execute`] (HIGH-1 gate enforcement); the daemon mints it
    /// only for an admitted call, so an `Execute` without a matching proof is
    /// refused. The gate itself returns `None`; the dispatcher fills it after the
    /// decision (and, for a confirm, after the consent broker resolves).
    Allow {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proof: Option<String>,
    },
    /// Refuse the tool; `reason` is shown to the model, never to a user as-is.
    Deny { reason: String },
    /// Run the tool, but with these daemon-substituted arguments instead. `proof`
    /// binds those substituted args (see [`Allow`]).
    Modify {
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proof: Option<String>,
    },
    /// Hold for an explicit user confirmation (see [`Confirm`]); `prompt` is the
    /// question the trusted-path consent surface asks.
    Confirm { prompt: String },
}

/// Engine to daemon: run a PRIVILEGED tool in trusted Rust (the daemon never
/// lets the engine touch the KG/OS directly). The daemon re-validates the args,
/// runs the real action, audits it, registers compensation, and returns only
/// the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execute {
    /// The privileged proxy tool to run.
    pub tool_name: String,
    /// The (already-authorized) arguments.
    pub tool_input: serde_json::Value,
    /// The one-time execution proof the matching [`Authorize`] minted (HIGH-1).
    /// The daemon validates and consumes it before running the tool; a missing,
    /// mismatched, reused, or expired proof is refused. Optional on the wire so the
    /// field is additive, but the daemon requires it (fail-closed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
}

/// The outcome of an [`Execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExecuteOutcome {
    /// The action ran; `result` is what the engine feeds back to the model.
    Ok { result: serde_json::Value },
    /// The action did not run; `code` classifies it, `message` is engine-facing.
    Error { code: ContractError, message: String },
}

/// Engine to daemon: a tool result, for audit + compensation registration +
/// S17/S18 screening BEFORE the content re-enters the engine's context. Maps to
/// pi `tool_result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The tool whose result this is.
    pub tool_name: String,
    /// The engine's id for the originating tool call (pairs result to call).
    pub tool_call_id: String,
    /// The tool result content (opaque to this contract).
    pub result: serde_json::Value,
    /// Whether the tool reported an error.
    pub is_error: bool,
}

/// The daemon's response to a [`Report`]: the screening verdict on the result's
/// content. A `Block` means the engine must drop the content (it never re-enters
/// the model's context); the structural gate holds even if screening fails open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAck {
    /// The S17/S18 screening verdict on the reported content.
    pub screen: ScreenVerdict,
}

/// The result of screening reported content for prompt-injection / sensitive
/// material (S17/S18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenVerdict {
    /// Nothing flagged; the content may re-enter the engine's context.
    Clean,
    /// Flagged but allowed through (logged); the gate's confirm-on-external
    /// trigger is the action-level containment.
    Warn,
    /// The content must NOT re-enter the engine's context.
    Block,
}

/// Daemon to the trusted-path consent surface: a confirmation that blocks for
/// the user's answer. Driven by the daemon (never a spoofable engine->shell
/// channel) when an [`AuthorizeDecision::Confirm`] is reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirm {
    /// The tool the confirmation is for.
    pub tool_name: String,
    /// The question shown on the trusted-path consent surface.
    pub prompt: String,
    /// How serious the action is (drives the surface's presentation).
    pub severity: ConfirmSeverity,
}

/// How serious a [`Confirm`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmSeverity {
    /// A routine confirmation.
    Normal,
    /// A high-impact action (permanent delete, external send, privilege change).
    High,
}

/// The user's answer to a [`Confirm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ConfirmAnswer {
    /// The user approved the action.
    Approved,
    /// The user declined (or the surface timed out / failed closed).
    Denied,
}

/// Why a contract operation failed. Part of the wire contract, so the names are
/// stable across daemon/engine versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractError {
    /// The named tool is not known to the daemon.
    UnknownTool,
    /// The session's grant does not cover the action.
    PermissionDenied,
    /// The arguments failed the daemon's re-validation.
    InvalidArguments,
    /// The action ran but failed.
    ExecutionFailed,
    /// A required daemon dependency (audit, consent) was unavailable; the action
    /// fails closed.
    Unavailable,
    /// An unexpected internal error.
    Internal,
}

/// One call the engine's plugin makes to the daemon over the contract socket.
/// `SessionInit` is daemon-driven (the daemon mints the token), so it is not a
/// `Call`; the engine echoes the minted token on every subsequent call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    /// Authorize a proposed tool call.
    Authorize(Authorize),
    /// Run a privileged tool in trusted Rust.
    Execute(Execute),
    /// Report a tool result for audit/compensation/screening.
    Report(Report),
    /// Tear the session down (the engine is exiting / the run ended).
    EndSession,
}

/// The engine-to-daemon contract message: the session token plus one [`Call`].
/// The daemon resolves the calling pid from SO_PEERCRED (never the wire) and
/// pairs it with this token to bound the action server-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractCall {
    /// The session token the daemon minted at SessionInit.
    pub token: String,
    /// The call being made.
    pub call: Call,
}

/// The daemon's reply to a [`ContractCall`]. The variant matches the call:
/// Authorize -> a decision, Execute -> an outcome, Report -> a screen ack,
/// EndSession -> `Ack`. `Error` is a contract-level failure (e.g. a malformed
/// call) distinct from an in-band Deny/Error the verb itself carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    /// The verdict for an `Authorize` call.
    Authorize(AuthorizeDecision),
    /// The outcome of an `Execute` call.
    Execute(ExecuteOutcome),
    /// The screen ack for a `Report` call.
    Report(ReportAck),
    /// Acknowledgement of an `EndSession` (or other no-result) call.
    Ack,
    /// A contract-level failure handling the call itself. A struct variant (not
    /// a newtype) so the wire form is the clean `{"reply":"error","code":...}`
    /// the plugins can read, matching the modulesd-proto `HostReply::Error`
    /// shape (a newtype around the string-enum serialized as `{"<code>":null}`,
    /// a serde artifact the TypeScript consumer cannot ergonomically decode).
    Error { code: ContractError },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T>(v: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(v).unwrap()).unwrap()
    }

    #[test]
    fn session_init_round_trips() {
        let s = SessionInit {
            system_prompt: "be helpful".into(),
            behaviour: Some("auto-tag".into()),
            capability_context: CapabilityContext {
                generic_tools: vec!["bash".into()],
                proxy_tools: vec!["graph.read".into(), "graph.write".into()],
            },
            project_anchor: Some("proj-1".into()),
            read_tier: ReadTier::Standard,
            externally_triggered: true,
        };
        assert_eq!(round_trip(&s), s);
    }

    /// `AuthorizeDecision` is the security-critical contract; its `decision` tag
    /// names must stay stable so a daemon and an older engine plugin agree.
    #[test]
    fn authorize_decision_tags_are_stable() {
        let cases: Vec<(AuthorizeDecision, &str)> = vec![
            (AuthorizeDecision::Allow { proof: None }, "allow"),
            (AuthorizeDecision::Deny { reason: "no".into() }, "deny"),
            (AuthorizeDecision::Modify { args: json!({"k": "v"}), proof: None }, "modify"),
            (AuthorizeDecision::Confirm { prompt: "ok?".into() }, "confirm"),
        ];
        for (d, tag) in cases {
            let s = serde_json::to_string(&d).unwrap();
            assert!(s.contains(&format!("\"decision\":\"{tag}\"")), "{tag}: tag changed, got {s}");
            assert_eq!(round_trip(&d), d);
        }
    }

    #[test]
    fn authorize_and_execute_preserve_opaque_json() {
        let a = Authorize {
            tool_name: "graph.write".into(),
            tool_input: json!({"cypher": "CREATE (n)", "nested": [1, 2, {"x": true}]}),
            external_triggered: true,
        };
        assert_eq!(round_trip(&a), a);

        let ok = ExecuteOutcome::Ok { result: json!({"rows": []}) };
        let s = serde_json::to_string(&ok).unwrap();
        assert!(s.contains("\"outcome\":\"ok\""));
        assert_eq!(round_trip(&ok), ok);

        let err = ExecuteOutcome::Error { code: ContractError::PermissionDenied, message: "denied".into() };
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("\"outcome\":\"error\""));
        assert!(s.contains("\"code\":\"permission_denied\""));
        assert_eq!(round_trip(&err), err);
    }

    #[test]
    fn report_and_screen_verdict_round_trip() {
        let r = Report {
            tool_name: "graph.read".into(),
            tool_call_id: "call-7".into(),
            result: json!({"text": "rows"}),
            is_error: false,
        };
        assert_eq!(round_trip(&r), r);

        for (v, name) in [
            (ScreenVerdict::Clean, "clean"),
            (ScreenVerdict::Warn, "warn"),
            (ScreenVerdict::Block, "block"),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), format!("\"{name}\""));
            assert_eq!(round_trip(&ReportAck { screen: v }).screen, v);
        }
    }

    /// The full `ContractCall` + `Reply` wire envelope, pinned exactly (not just
    /// round-tripped): the thin pi plugins are a separate (TypeScript) consumer
    /// that hand-builds this JSON, so a serde change that silently reshaped the
    /// envelope would break them with no Rust failure. `to_value` compares the
    /// complete object, so an added/renamed/removed field fails here.
    #[test]
    fn contract_call_and_reply_wire_envelope_is_pinned() {
        let call = ContractCall {
            token: "tok-1".into(),
            call: Call::Authorize(Authorize {
                tool_name: "graph.write".into(),
                tool_input: json!({"cypher": "CREATE (n)"}),
                external_triggered: false,
            }),
        };
        assert_eq!(
            serde_json::to_value(&call).unwrap(),
            json!({
                "token": "tok-1",
                "call": {
                    "call": "authorize",
                    "tool_name": "graph.write",
                    "tool_input": {"cypher": "CREATE (n)"},
                    "external_triggered": false
                }
            }),
        );

        // EndSession is a unit variant: just the tag.
        let end = ContractCall { token: "t".into(), call: Call::EndSession };
        assert_eq!(
            serde_json::to_value(&end).unwrap(),
            json!({"token": "t", "call": {"call": "end_session"}}),
        );

        // The reply envelope nests the verb's tagged verdict under the `reply` tag.
        assert_eq!(
            serde_json::to_value(Reply::Authorize(AuthorizeDecision::Allow { proof: None })).unwrap(),
            json!({"reply": "authorize", "decision": "allow"}),
        );
        assert_eq!(
            serde_json::to_value(Reply::Report(ReportAck { screen: ScreenVerdict::Block })).unwrap(),
            json!({"reply": "report", "screen": "block"}),
        );
        assert_eq!(serde_json::to_value(Reply::Ack).unwrap(), json!({"reply": "ack"}));
        assert_eq!(
            serde_json::to_value(Reply::Error { code: ContractError::Unavailable }).unwrap(),
            json!({"reply": "error", "code": "unavailable"}),
        );

        // The whole envelope round-trips too (the plugin's decode path).
        assert_eq!(round_trip(&call), call);
    }

    #[test]
    fn confirm_round_trips_with_stable_names() {
        let c = Confirm {
            tool_name: "fs.delete".into(),
            prompt: "Permanently delete?".into(),
            severity: ConfirmSeverity::High,
        };
        assert_eq!(round_trip(&c), c);
        assert_eq!(serde_json::to_string(&ConfirmSeverity::High).unwrap(), "\"high\"");
        let s = serde_json::to_string(&ConfirmAnswer::Approved).unwrap();
        assert!(s.contains("\"answer\":\"approved\""));
        assert_eq!(round_trip(&ConfirmAnswer::Denied), ConfirmAnswer::Denied);
    }

    #[test]
    fn read_tier_and_error_names_are_stable() {
        for (t, name) in [
            (ReadTier::None, "none"),
            (ReadTier::Minimal, "minimal"),
            (ReadTier::Standard, "standard"),
            (ReadTier::Extended, "extended"),
            (ReadTier::Full, "full"),
        ] {
            assert_eq!(serde_json::to_string(&t).unwrap(), format!("\"{name}\""));
        }
        assert_eq!(serde_json::to_string(&ContractError::UnknownTool).unwrap(), "\"unknown_tool\"");
        assert_eq!(serde_json::to_string(&ContractError::Unavailable).unwrap(), "\"unavailable\"");
    }

    #[test]
    fn contract_call_round_trips_and_carries_the_token() {
        let c = ContractCall {
            token: "abc123".into(),
            call: Call::Authorize(Authorize {
                tool_name: "bash".into(),
                tool_input: json!({"command": "ls"}),
                external_triggered: false,
            }),
        };
        let back = round_trip(&c);
        assert_eq!(back.token, "abc123");
        assert!(matches!(back.call, Call::Authorize(_)));
        assert_eq!(back, c);
    }

    /// The `call`/`reply` tags are the wire contract between the engine plugin
    /// and the daemon; a renamed variant must fail a test, not break IPC.
    #[test]
    fn call_and_reply_tags_are_stable() {
        let calls: Vec<(Call, &str)> = vec![
            (Call::Authorize(Authorize { tool_name: "t".into(), tool_input: json!(null), external_triggered: false }), "authorize"),
            (Call::Execute(Execute { tool_name: "t".into(), tool_input: json!(null), proof: None }), "execute"),
            (Call::Report(Report { tool_name: "t".into(), tool_call_id: "c".into(), result: json!(null), is_error: false }), "report"),
            (Call::EndSession, "end_session"),
        ];
        for (c, tag) in calls {
            let s = serde_json::to_string(&c).unwrap();
            assert!(s.contains(&format!("\"call\":\"{tag}\"")), "{tag}: call tag changed, got {s}");
        }
        let replies: Vec<(Reply, &str)> = vec![
            (Reply::Authorize(AuthorizeDecision::Allow { proof: None }), "authorize"),
            (Reply::Execute(ExecuteOutcome::Ok { result: json!(null) }), "execute"),
            (Reply::Report(ReportAck { screen: ScreenVerdict::Clean }), "report"),
            (Reply::Ack, "ack"),
            (Reply::Error { code: ContractError::Internal }, "error"),
        ];
        for (r, tag) in replies {
            let s = serde_json::to_string(&r).unwrap();
            assert!(s.contains(&format!("\"reply\":\"{tag}\"")), "{tag}: reply tag changed, got {s}");
            assert_eq!(round_trip(&r), r);
        }
    }
}
