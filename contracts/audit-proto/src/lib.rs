//! Shared wire protocol for the Arlen audit daemon.
//!
//! Both the audit daemon (`arlen-auditd`) and its clients (the AI
//! daemon, the AI network proxy) depend on this crate so the ingest
//! types have a single definition and cannot drift. The crate is
//! deliberately thin — serde types, length-prefixed framing, and a
//! small connect-per-submit [`client`] — so an audit *client* does
//! not pull in the daemon's SQLite or crypto dependencies.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub mod activity;
pub mod client;
pub mod read;
pub mod read_client;
pub mod sink;

pub use activity::{ActivityEntry, ActivityPage, MAX_ACTIVITY_LIMIT};
pub use read::{read_socket_path, ReadPage, ReadRequest, ReadResponse, StructuralView};
pub use read_client::{ReadClient, ReadClientError};

pub use sink::{AuditSink, LedgerAuditSink};

// The in-memory mock is a fail-open sink (it returns success without a
// ledger), so it is gated behind `test-util` and never present in a
// production build.
#[cfg(any(test, feature = "test-util"))]
pub use sink::MockAuditSink;

// Defence in depth for the gate above: `test-util` is "off by default"
// but a feature is globally unifiable, so `--all-features` or a stray
// dependency could otherwise pull the fail-open mock into a real
// build. An optimized build (release / production, `debug_assertions`
// off) with `test-util` enabled is therefore a hard compile error, so
// the mock can never reach a shipped binary; tests run in the dev
// profile (`debug_assertions` on) and are unaffected.
#[cfg(all(feature = "test-util", not(debug_assertions)))]
compile_error!(
    "audit-proto's `test-util` feature exposes the fail-open MockAuditSink and is \
     for tests only; it must not be enabled in an optimized/release build. Remove \
     `--all-features` from the release build or scope the feature to dev-dependencies."
);

/// Largest accepted frame. An audit event is small; this only bounds
/// a malformed or hostile sender. Enforced symmetrically on both
/// [`read_frame`] and [`write_frame`].
pub const MAX_FRAME: usize = 256 * 1024;

/// Field-size caps for [`StructuralRecord`], enforced by
/// [`IngestRequest::validate`]. The Structural tier holds coarse
/// identifiers (a server id, a tool name, a host, a fixed label) and
/// counts — never free-form content — so these bounds are generous
/// for legitimate use and a backstop against a caller that smuggles a
/// query string, a result blob, or many fabricated entries into the
/// always-recorded tier.
pub const MAX_SUBJECT_LEN: usize = 256;
/// Max length of the coarse `outcome` label (e.g. `forwarded-200`).
pub const MAX_OUTCOME_LEN: usize = 128;
/// Max length of each graph node-type / relation label.
pub const MAX_LABEL_LEN: usize = 128;
/// Max number of node-type / relation labels in one entry.
pub const MAX_LABELS: usize = 64;
/// Max length of a call-chain id or project id.
pub const MAX_ID_LEN: usize = 256;

/// Errors from framing and protocol encoding.
#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    /// Underlying socket I/O failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A frame whose declared length is out of range.
    #[error("frame: {0}")]
    Frame(String),
    /// A request or response that could not be (de)serialised.
    #[error("codec: {0}")]
    Codec(String),
    /// A request that violates a field-size cap. The audit daemon
    /// rejects it before append (fail-closed for the caller).
    #[error("invalid request: {0}")]
    Invalid(String),
}

/// Result alias for protocol operations.
pub type Result<T> = std::result::Result<T, ProtoError>;

/// What kind of audited action an entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    /// An AI natural-language query.
    Query,
    /// An MCP tool invocation.
    ToolCall,
    /// A user confirmation of a high-impact action.
    Confirm,
    /// A rejected call: depth limit, permission, or always-confirm.
    PolicyViolation,
    /// A Knowledge-Graph access.
    GraphAccess,
    /// A permission grant or denial.
    Permission,
    /// An outbound call to an AI provider, made via the AI network
    /// proxy.
    NetworkCall,
    /// A non-AI system action by an app or daemon worth recording but
    /// outside the AI taxonomy: a notification surfaced to the user, a
    /// package installed or removed, a file trashed. Carries only coarse
    /// identifiers (the acting/posting app and a disposition), never the
    /// action's content.
    AppAction,
    /// A capability change: a user (or the Settings app) revoked, restored, or
    /// granted a reach. The coarse `outcome` label is the change
    /// (`revoked` / `restored` / `granted`); the specific reach rides the typed
    /// [`StructuralRecord::capability_change`] field. This is the one record class
    /// that carries a reach, because a reach is authority-metadata (ISO-27560
    /// consent-receipt provenance), not user content - the S13 content-free
    /// boundary still holds for every data-access record.
    CapabilityChange,
}

impl AuditKind {
    /// Stable wire string. Also the suffix of the Event Bus event
    /// type re-emitted after a successful append (`audit.ai.<kind>`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::ToolCall => "tool_call",
            Self::Confirm => "confirm",
            Self::PolicyViolation => "policy_violation",
            Self::GraphAccess => "graph_access",
            Self::Permission => "permission",
            Self::NetworkCall => "network_call",
            Self::AppAction => "app_action",
            Self::CapabilityChange => "capability_change",
        }
    }

    /// Parse a kind from its [`as_str`](Self::as_str) wire form.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "query" => Some(Self::Query),
            "tool_call" => Some(Self::ToolCall),
            "confirm" => Some(Self::Confirm),
            "policy_violation" => Some(Self::PolicyViolation),
            "graph_access" => Some(Self::GraphAccess),
            "permission" => Some(Self::Permission),
            "network_call" => Some(Self::NetworkCall),
            "app_action" => Some(Self::AppAction),
            "capability_change" => Some(Self::CapabilityChange),
            _ => None,
        }
    }
}

/// The reach a [`AuditKind::CapabilityChange`] record carries: the specific
/// capability that was revoked, restored, or granted. Authority-metadata (a
/// capability pattern / relation, never user content), so it is safe to record in
/// the ledger and is the durable "what was removed" the profile-first restore reads
/// back (living-capability-graph.md §6, Tim's 1-July audit-ledger decision).
///
/// A typed mirror of the SDK `arlen_permissions::revoke::RevokedReach` variants,
/// kept audit-proto-local so this low-level wire crate stays dependency-light (a
/// producer converts its own reach type into this at the ingest boundary).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReach {
    /// A `[graph].read` entity-type pattern.
    Read {
        /// The read pattern.
        entity_pattern: String,
    },
    /// A `[graph].write` entity-type pattern.
    Write {
        /// The write pattern.
        entity_pattern: String,
    },
    /// A `[graph].relations` entry.
    Relation {
        /// The relation's source entity type.
        from: String,
        /// The relation's target entity type.
        to: String,
        /// The relation type.
        relation_type: String,
    },
    /// The `instance_scope = all` cross-app reach.
    InstanceAll,
    /// A `[network].allowed_domains` egress domain.
    NetworkDomain {
        /// The network domain.
        domain: String,
    },
    /// A `[clipboard]` capability flag (read/write/read_sensitive/history).
    ClipboardCap {
        /// The clipboard capability.
        cap: String,
    },
    /// The `[notifications].enabled` single-flag dimension.
    NotificationsOff,
    /// An `[input]` capability flag (focused/global keybinding registration).
    InputCap {
        /// The input capability.
        cap: String,
    },
}

/// Content-free interaction metadata — the Structural tier of
/// foundation §8.4.7, always recorded. Every field is a coarse
/// identifier or a count; none can hold a query string, a result
/// value, or a concrete node ID (the one class-scoped exception is
/// [`capability_change`](Self::capability_change), which carries an
/// authority-metadata reach for a capability-change record only).
///
/// `Default` is derived so a producer can build a record with the fields it sets
/// plus `..Default::default()`, and so future additive fields do not churn every
/// construction site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StructuralRecord {
    /// Coarse subject identifier: an MCP server id, a tool name, or
    /// a graph target label. Never free-form content.
    pub subject: String,
    /// Graph node types touched, if any.
    #[serde(default)]
    pub node_types: Vec<String>,
    /// Graph relations traversed, if any.
    #[serde(default)]
    pub relations: Vec<String>,
    /// Number of results, when meaningful for this kind.
    #[serde(default)]
    pub result_count: Option<u64>,
    /// Wall-clock duration of the action, when measured.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Coarse outcome label: `ok`, `denied`, `error`, ...
    pub outcome: String,
    /// MCP call-chain depth, when the entry is part of one.
    #[serde(default)]
    pub depth: Option<u8>,
    /// The specific reach, for an [`AuditKind::CapabilityChange`] record only: the
    /// capability revoked / restored / granted (the `outcome` label says which).
    /// `None` for every other kind - the S13 content-free boundary holds for
    /// data-access records; only the capability-change class carries a reach, and a
    /// reach is authority-metadata, not user content.
    #[serde(default)]
    pub capability_change: Option<CapabilityReach>,
}

/// The opt-in Forensic tier (foundation §8.4.7): the content the
/// Structural tier deliberately omits. An entry carries this only
/// when Forensic Mode was active when it was written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForensicRecord {
    /// The full query string.
    pub query_string: String,
    /// Query parameters (not result contents).
    pub parameters: String,
    /// The calling process stack trace.
    pub stack_trace: String,
}

/// One audit event submitted over the ingest socket.
///
/// Note what this does **not** carry: the `actor`. The audit daemon
/// sets the actor from the connection's kernel-attested peer identity
/// (`SO_PEERCRED`), never from the request, so a caller cannot
/// misattribute an entry to another component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    /// What kind of action is being recorded.
    pub kind: AuditKind,
    /// Content-free structural metadata (always recorded).
    pub structural: StructuralRecord,
    /// Optional Forensic-tier content, present only when Forensic
    /// Mode is active.
    #[serde(default)]
    pub forensic: Option<ForensicRecord>,
    /// MCP call-chain id, when the event belongs to one.
    #[serde(default)]
    pub call_chain_id: Option<String>,
    /// Project context, when one was active.
    #[serde(default)]
    pub project_id: Option<String>,
}

impl IngestRequest {
    /// Reject a request whose Structural fields exceed the size caps.
    ///
    /// The audit daemon calls this before append, so it does not trust
    /// the caller to keep the always-recorded Structural tier coarse:
    /// an oversized `subject` or `outcome`, or an unreasonable number
    /// of labels, is refused rather than persisted. This is a backstop
    /// against content (a query string, a result blob) being smuggled
    /// into the daemon-readable tier; the content-free *shape* is
    /// provided by the producer-side builders, this enforces the
    /// *size* server-side. The Forensic tier is deliberately not
    /// capped here — it is the opt-in content tier, read only by the
    /// user's own session.
    pub fn validate(&self) -> Result<()> {
        let s = &self.structural;
        if s.subject.len() > MAX_SUBJECT_LEN {
            return Err(ProtoError::Invalid(format!(
                "subject exceeds {MAX_SUBJECT_LEN} bytes"
            )));
        }
        if s.outcome.len() > MAX_OUTCOME_LEN {
            return Err(ProtoError::Invalid(format!(
                "outcome exceeds {MAX_OUTCOME_LEN} bytes"
            )));
        }
        if s.node_types.len() > MAX_LABELS || s.relations.len() > MAX_LABELS {
            return Err(ProtoError::Invalid(format!(
                "more than {MAX_LABELS} labels"
            )));
        }
        for label in s.node_types.iter().chain(s.relations.iter()) {
            if label.len() > MAX_LABEL_LEN {
                return Err(ProtoError::Invalid(format!(
                    "a label exceeds {MAX_LABEL_LEN} bytes"
                )));
            }
        }
        for id in [&self.call_chain_id, &self.project_id].into_iter().flatten() {
            if id.len() > MAX_ID_LEN {
                return Err(ProtoError::Invalid(format!(
                    "an id exceeds {MAX_ID_LEN} bytes"
                )));
            }
        }
        Ok(())
    }
}

/// The audit daemon's reply to an [`IngestRequest`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IngestResponse {
    /// The entry was appended at this chain index.
    Appended {
        /// The assigned chain index.
        index: u64,
    },
    /// The entry could not be recorded. The caller must fail closed:
    /// per foundation §8.4.6 there is no un-audited AI activity.
    Unavailable {
        /// Human-readable reason, for the caller's log.
        reason: String,
    },
}

/// Resolve the ingest socket path:
/// `$XDG_RUNTIME_DIR/arlen/audit-ingest.sock`, falling back to
/// `/run/arlen/audit-ingest.sock`.
pub fn ingest_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("audit-ingest.sock")
}

/// Read one length-prefixed frame from `stream`.
pub async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(ProtoError::Frame(format!(
            "frame length {len} out of range (1..={MAX_FRAME})"
        )));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

/// Write one length-prefixed frame to `stream`.
///
/// Enforces the same bounds as [`read_frame`]: an empty body or one
/// past [`MAX_FRAME`] is rejected before any bytes are written, so the
/// writer can never emit a frame the reader would refuse, and an
/// oversized payload becomes an immediate protocol error rather than a
/// large transfer the peer then rejects.
pub async fn write_frame<S>(stream: &mut S, body: &[u8]) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(ProtoError::Frame(format!(
            "frame length {} out of range (1..={MAX_FRAME})",
            body.len()
        )));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| ProtoError::Frame("frame exceeds u32 length".to_string()))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

/// Encode an [`IngestRequest`] to a frame body.
pub fn encode_request(req: &IngestRequest) -> Result<Vec<u8>> {
    serde_json::to_vec(req)
        .map_err(|e| ProtoError::Codec(format!("encode request: {e}")))
}

/// Decode an [`IngestRequest`] from a frame body.
pub fn decode_request(body: &[u8]) -> Result<IngestRequest> {
    serde_json::from_slice(body)
        .map_err(|e| ProtoError::Codec(format!("decode request: {e}")))
}

/// Encode an [`IngestResponse`] to a frame body.
pub fn encode_response(resp: &IngestResponse) -> Result<Vec<u8>> {
    serde_json::to_vec(resp)
        .map_err(|e| ProtoError::Codec(format!("encode response: {e}")))
}

/// Decode an [`IngestResponse`] from a frame body.
pub fn decode_response(body: &[u8]) -> Result<IngestResponse> {
    serde_json::from_slice(body)
        .map_err(|e| ProtoError::Codec(format!("decode response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_wire_strings_round_trip() {
        for kind in [
            AuditKind::Query,
            AuditKind::ToolCall,
            AuditKind::Confirm,
            AuditKind::PolicyViolation,
            AuditKind::GraphAccess,
            AuditKind::Permission,
            AuditKind::NetworkCall,
            AuditKind::AppAction,
        ] {
            assert_eq!(AuditKind::from_wire(kind.as_str()), Some(kind));
        }
        assert_eq!(AuditKind::from_wire("nonsense"), None);
    }

    #[test]
    fn request_and_response_round_trip() {
        let req = IngestRequest {
            kind: AuditKind::ToolCall,
            structural: StructuralRecord {
                subject: "com.arlen.files".into(),
                node_types: vec![],
                relations: vec![],
                result_count: None,
                duration_ms: Some(4),
                outcome: "ok".into(),
                depth: Some(2),
                capability_change: None,
            },
            forensic: None,
            call_chain_id: Some("chain-7".into()),
            project_id: None,
        };
        let back = decode_request(&encode_request(&req).unwrap()).unwrap();
        assert_eq!(back.kind, AuditKind::ToolCall);
        assert_eq!(back.call_chain_id.as_deref(), Some("chain-7"));

        let resp = IngestResponse::Appended { index: 12 };
        assert_eq!(decode_response(&encode_response(&resp).unwrap()).unwrap(), resp);
    }

    #[tokio::test]
    async fn frame_round_trips_and_rejects_oversize() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        write_frame(&mut a, b"audit").await.unwrap();
        assert_eq!(read_frame(&mut b).await.unwrap(), b"audit");

        let (mut c, mut d) = tokio::io::duplex(16);
        let bogus = ((MAX_FRAME + 1) as u32).to_be_bytes();
        c.write_all(&bogus).await.unwrap();
        assert!(read_frame(&mut d).await.is_err());
    }

    #[tokio::test]
    async fn write_frame_enforces_the_same_bounds_as_read() {
        // The writer must refuse what the reader would refuse: empty
        // and oversized bodies, before any byte is sent.
        let (mut a, _b) = tokio::io::duplex(64);
        assert!(write_frame(&mut a, b"").await.is_err(), "empty body rejected");
        let oversized = vec![0u8; MAX_FRAME + 1];
        assert!(
            write_frame(&mut a, &oversized).await.is_err(),
            "oversized body rejected"
        );
    }

    fn structural(subject: &str, outcome: &str) -> StructuralRecord {
        StructuralRecord {
            subject: subject.into(),
            node_types: vec![],
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: outcome.into(),
            depth: None,
            capability_change: None,
        }
    }

    fn request(structural: StructuralRecord) -> IngestRequest {
        IngestRequest {
            kind: AuditKind::Query,
            structural,
            forensic: None,
            call_chain_id: None,
            project_id: None,
        }
    }

    #[test]
    fn validate_accepts_normal_coarse_metadata() {
        assert!(request(structural("ai.query", "completed")).validate().is_ok());
    }

    #[test]
    fn validate_rejects_an_oversized_subject() {
        // A subject big enough to be smuggled content, not an
        // identifier, is refused.
        let big = "x".repeat(MAX_SUBJECT_LEN + 1);
        let err = request(structural(&big, "ok")).validate().unwrap_err();
        assert!(matches!(err, ProtoError::Invalid(_)), "got: {err:?}");
    }

    #[test]
    fn validate_rejects_too_many_labels() {
        let mut s = structural("ai.query", "ok");
        s.node_types = (0..(MAX_LABELS + 1)).map(|i| format!("L{i}")).collect();
        assert!(matches!(
            request(s).validate().unwrap_err(),
            ProtoError::Invalid(_)
        ));
    }
}
