//! Peer-authenticated ingest socket.
//!
//! Other components submit audit events here; the audit daemon stays
//! the sole writer of the ledger (foundation §8.4.7). Each connection
//! is authenticated via `SO_PEERCRED` — only the AI-layer daemons are
//! admitted — and the entry's `actor` is set from that kernel-attested
//! identity, never from the request, so a caller cannot misattribute.
//!
//! Known limitation (shared, documented): admission rests on the
//! `app_id` that `arlen-permissions` resolves from the peer's
//! binary. That resolver has an open same-uid gap — a user-installed
//! app under `~/.local/share/arlen/apps/{app_id}/` resolves to
//! `{app_id}`, so a same-uid process could install itself as
//! `ai-daemon` and pass [`ADMITTED`]. This is the F3 gap tracked in
//! `docs/architecture/identity-spoof-mitigation.md`; the global fix
//! is the installd inode-keyed identity registry, which closes it for
//! every peer-authenticated broker at once, not just this one. Until
//! then a same-uid compromise can forge entries — the same trust
//! boundary that lets a same-uid process read the HMAC key. The
//! hardware-rooted closers (TPM, installd registry) are foundation
//! §8.4 hardening follow-ups.
//!
//! The append is synchronous and acknowledged: the caller learns
//! whether its event was recorded and, per foundation §8.4.6, fails
//! closed if it was not. After a committed append the daemon re-emits
//! an `audit.ai.<kind>` event on the Event Bus carrying the chain
//! index, for the Anomaly Detector.

pub use audit_proto::{IngestRequest, IngestResponse};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use audit_proto::{decode_request, encode_response, read_frame, write_frame};
use arlen_permissions::ConnectionAuth;
use os_sdk::{EventEmitter, UnixEventEmitter};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::error::{AuditError, Result};
use crate::ledger::Ledger;

/// app_ids permitted to submit audit events. Phase 9 audits the AI
/// layer: `ai-daemon` and `ai-proxy` audit their model calls, and
/// `ai-agent` audits every gate decision before acting (the
/// fail-closed audit-before-act in `LiveExecutor`). The agent MUST be
/// here or in a release build `caller_is_admitted` returns false, the
/// `LedgerAuditSink` reports `Unavailable`, and the gate refuses every
/// action — the agent cannot act at all. Debug builds masked this
/// because the agent then resolves to a `dev.*` id (admitted below).
/// `knowledge` (the graph daemon) audits each app-tier entity upsert
/// (the foreign-app-bridges write path), fail-closed before persisting.
/// `online-accounts` audits each credential handout (`GetAccessToken`,
/// GAP-2), `notifyd` audits each notification disposition, and `installd`
/// audits each install/uninstall (GAP-2); all resolve to these ids via their
/// canonical libexec path entries in `path_to_app_id`.
const ADMITTED: &[&str] = &[
    "ai-daemon",
    "ai-proxy",
    "ai-agent",
    "online-accounts",
    "notifyd",
    "installd",
    "knowledge",
    "consent-broker",
];

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

/// The ingest server: shares the single [`Ledger`] writer across
/// connections and re-emits `audit.ai.*` on the Event Bus.
pub struct IngestServer {
    ledger: Arc<Mutex<Ledger>>,
    emitter: Arc<UnixEventEmitter>,
    /// Set when startup chain verification found the ledger tampered.
    /// While set, every append is refused with `Unavailable` so the
    /// caller fails closed: a tampered ledger must not silently
    /// accept new entries chaining onto a broken history.
    tampered: Arc<AtomicBool>,
}

impl IngestServer {
    /// Build a server over a shared ledger, an Event Bus emitter, and
    /// the shared tamper flag. The daemon sets the flag before
    /// serving if the chain failed startup verification.
    pub fn new(
        ledger: Arc<Mutex<Ledger>>,
        emitter: Arc<UnixEventEmitter>,
        tampered: Arc<AtomicBool>,
    ) -> Self {
        Self {
            ledger,
            emitter,
            tampered,
        }
    }

    /// Bind the ingest socket and serve it until the accept loop
    /// errors. The daemon spawns this as its long-lived task.
    pub async fn run(self: Arc<Self>, socket_path: &Path) -> Result<()> {
        let listener = crate::bind_unix_socket(socket_path)?;
        let caller_uid = current_uid();
        tracing::info!(socket = %socket_path.display(), "audit ingest listening");
        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|e| AuditError::Storage(format!("ingest accept: {e}")))?;
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = server.handle(stream, caller_uid).await {
                    tracing::warn!("ingest connection error: {e}");
                }
            });
        }
    }

    /// Handle one connection: authenticate the peer, then field
    /// ingest requests until it closes.
    async fn handle(&self, stream: UnixStream, caller_uid: u32) -> Result<()> {
        let auth = match ConnectionAuth::extract_from(&stream, caller_uid) {
            Ok(auth) => auth,
            Err(e) => {
                tracing::warn!("ingest connection rejected: peer identity: {e}");
                return Ok(());
            }
        };
        if !caller_is_admitted(auth.app_id()) {
            tracing::warn!(
                caller = %auth.app_id(),
                "ingest connection rejected: caller is not admitted"
            );
            return Ok(());
        }
        // The actor is the kernel-attested peer identity, never a
        // request field — a caller cannot record under another name.
        let actor = auth.app_id().to_string();
        self.serve_connection(stream, &actor).await
    }

    /// Field ingest requests on an already-authenticated connection
    /// until the peer closes. `actor` is the kernel-attested identity
    /// recorded on every entry. Split out of [`handle`](Self::handle)
    /// so the transport + record + ledger round-trip is testable
    /// without re-deriving admission from the test process's own pid.
    async fn serve_connection(&self, mut stream: UnixStream, actor: &str) -> Result<()> {
        loop {
            let body = match read_frame(&mut stream).await {
                Ok(body) => body,
                // A closed connection or a framing error ends the
                // session; the peer is a trusted daemon, nothing to
                // recover.
                Err(_) => return Ok(()),
            };
            let response = self.record(actor, &body).await;
            let encoded = encode_response(&response)?;
            write_frame(&mut stream, &encoded).await?;
        }
    }

    /// Decode, append, and — on a committed append — re-emit one
    /// audit event.
    async fn record(&self, actor: &str, body: &[u8]) -> IngestResponse {
        // A ledger that failed startup verification is frozen: no new
        // entry may chain onto a broken history. The caller fails
        // closed exactly as it would on a full disk.
        if self.tampered.load(Ordering::SeqCst) {
            return IngestResponse::Unavailable {
                reason: "audit ledger failed integrity verification; \
                         refusing new entries"
                    .to_string(),
            };
        }
        let req = match decode_request(body) {
            Ok(req) => req,
            Err(e) => {
                return IngestResponse::Unavailable {
                    reason: e.to_string(),
                }
            }
        };
        // Don't trust the caller to keep the always-recorded Structural
        // tier coarse: reject an event whose fields exceed the size
        // caps before it reaches the ledger (a backstop against content
        // smuggled into the daemon-readable tier).
        if let Err(e) = req.validate() {
            return IngestResponse::Unavailable {
                reason: e.to_string(),
            };
        }
        let appended = {
            let mut ledger = self.ledger.lock().await;
            ledger
                .append(
                    req.kind,
                    actor,
                    &req.structural,
                    req.forensic.as_ref(),
                    req.call_chain_id.as_deref(),
                    req.project_id.as_deref(),
                )
                .await
        };
        match appended {
            Ok(index) => {
                // Best-effort Event Bus notification. The ledger is
                // the source of truth; a bus failure does not undo a
                // committed entry, so the append is not rolled back.
                let event_type = format!("audit.ai.{}", req.kind.as_str());
                let payload =
                    serde_json::to_vec(&serde_json::json!({ "index": index }))
                        .unwrap_or_default();
                let _ = self.emitter.emit(&event_type, payload).await;
                IngestResponse::Appended { index }
            }
            // A full disk, a storage failure — anything — fails the
            // caller closed: it must not proceed with un-audited work.
            Err(e) => IngestResponse::Unavailable {
                reason: e.to_string(),
            },
        }
    }
}

/// The cargo-run `dev.*` ids of the admitted producers, accepted only
/// in debug builds. An exact list, not a broad `dev.` prefix: any
/// cargo-run crate resolves to some `dev.<bin>`, so a prefix match
/// would admit every locally-built binary to the ingest socket. Each
/// entry is `dev.<bin-name>` for the corresponding [`ADMITTED`] daemon.
#[cfg(debug_assertions)]
const DEV_ADMITTED: &[&str] = &[
    "dev.arlen-ai-daemon",
    "dev.arlen-ai-proxy",
    "dev.arlen-ai-agent",
    "dev.arlen-graph-daemon",
    "dev.arlen-consent-broker",
];

/// Whether a resolved peer app_id may submit audit events.
fn caller_is_admitted(app_id: &str) -> bool {
    if ADMITTED.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        DEV_ADMITTED.contains(&app_id) || dev_extra_admits(app_id)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// A debug-only test affordance: an integration harness sets
/// `ARLEN_AUDIT_EXTRA_ADMIT` to ONE extra dev id (its own cargo-run
/// `dev.<test>` id, which is hash-suffixed and so cannot be a static
/// [`DEV_ADMITTED`] entry) so it can exercise the ingest path as itself.
/// An EXACT match, never a broad `dev.` prefix, and never compiled into a
/// release build — normal dev keeps the tightened producer-only allowlist.
#[cfg(debug_assertions)]
fn dev_extra_admits(app_id: &str) -> bool {
    std::env::var("ARLEN_AUDIT_EXTRA_ADMIT").is_ok_and(|v| v == app_id)
}

/// The daemon's own uid, for `ConnectionAuth` peer extraction.
#[allow(unsafe_code)]
fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{AuditKind, StructuralRecord};

    #[test]
    fn admission_is_restricted_to_the_ai_layer() {
        assert!(caller_is_admitted("ai-daemon"));
        assert!(caller_is_admitted("ai-proxy"));
        assert!(caller_is_admitted("ai-agent"));
        // The graph daemon audits app-tier entity upserts (bridges write path).
        assert!(caller_is_admitted("knowledge"));
        assert!(!caller_is_admitted("com.example.app"));
        assert!(!caller_is_admitted(""));
        // Debug builds admit the listed cargo-run `dev.*` producer
        // ids, but not an arbitrary `dev.*` crate.
        assert_eq!(
            caller_is_admitted("dev.arlen-ai-daemon"),
            cfg!(debug_assertions)
        );
        assert_eq!(
            caller_is_admitted("dev.arlen-ai-agent"),
            cfg!(debug_assertions)
        );
        // The graph daemon's cargo-run bin is `arlen-graph-daemon`.
        assert_eq!(
            caller_is_admitted("dev.arlen-graph-daemon"),
            cfg!(debug_assertions)
        );
        assert!(!caller_is_admitted("dev.arlen-knowledge"));
        assert!(!caller_is_admitted("dev.evil"));
    }

    /// Every AI-layer producer that submits audit entries must be in
    /// the const allowlist itself, NOT reachable only through the
    /// debug `dev.*` fallback. This is a release-build guard: it would
    /// catch the AL-1 regression where `ai-agent` was omitted and its
    /// fail-closed gate audit silently broke in release while debug
    /// masked it via the `dev.*` admission. Asserting against
    /// `ADMITTED` directly is independent of the build profile.
    #[test]
    fn audit_producers_are_admitted_in_release() {
        for producer in [
            "ai-daemon",
            "ai-proxy",
            "ai-agent",
            "online-accounts",
            "notifyd",
            "installd",
        ] {
            assert!(
                ADMITTED.contains(&producer),
                "{producer} must be in ADMITTED, not rely on the debug dev.* fallback"
            );
        }
    }

    #[test]
    fn ingest_socket_path_is_under_arlen() {
        let p = ingest_socket_path();
        assert!(
            p.to_string_lossy().ends_with("arlen/audit-ingest.sock"),
            "{}",
            p.display()
        );
    }

    /// Drive the transport + record + ledger round-trip over a socket
    /// pair as an admitted producer (`ai-agent`), and confirm the event
    /// is appended and the ledger verifies. Admission itself is covered
    /// by [`admission_is_restricted_to_the_ai_layer`]; this exercises
    /// `serve_connection` for an already-authenticated peer rather than
    /// re-deriving admission from the test process's own pid (which is
    /// not a producer and is correctly refused after the dev.* tighten).
    #[tokio::test]
    async fn an_admitted_caller_appends_through_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(
            &dir.path().join("ledger.db"),
            zeroize::Zeroizing::new(b"test-key".to_vec()),
        )
            .await
            .expect("open ledger");
        let ledger = Arc::new(Mutex::new(ledger));
        // The emitter points at a nonexistent socket; emits fail and
        // are swallowed, which is the documented best-effort behaviour.
        let emitter = Arc::new(UnixEventEmitter::new("/nonexistent/producer.sock"));
        let server = Arc::new(IngestServer::new(
            Arc::clone(&ledger),
            emitter,
            Arc::new(AtomicBool::new(false)),
        ));

        let (mut client, server_end) = UnixStream::pair().unwrap();
        let serving = tokio::spawn(async move {
            let _ = server.serve_connection(server_end, "ai-agent").await;
        });

        let req = IngestRequest {
            kind: AuditKind::Query,
            structural: StructuralRecord {
                subject: "graph".into(),
                node_types: vec!["File".into()],
                relations: vec![],
                result_count: Some(2),
                duration_ms: Some(5),
                outcome: "ok".into(),
                depth: None,
                capability_change: None,
            },
            forensic: None,
            call_chain_id: None,
            project_id: None,
        };
        let body = serde_json::to_vec(&req).unwrap();
        write_frame(&mut client, &body).await.unwrap();

        let reply = read_frame(&mut client).await.unwrap();
        let resp: IngestResponse = serde_json::from_slice(&reply).unwrap();
        assert_eq!(resp, IngestResponse::Appended { index: 0 });

        // The entry is really in the ledger and the chain holds.
        assert_eq!(ledger.lock().await.verify().await.unwrap(), 1);

        drop(client);
        let _ = serving.await;
    }

    /// A server whose tamper flag is set must refuse every append:
    /// no entry may chain onto a ledger that failed verification.
    #[tokio::test]
    async fn a_tampered_ledger_refuses_appends() {
        let dir = tempfile::tempdir().unwrap();

        let ledger = Ledger::open(
            &dir.path().join("ledger.db"),
            zeroize::Zeroizing::new(b"test-key".to_vec()),
        )
            .await
            .expect("open ledger");
        let ledger = Arc::new(Mutex::new(ledger));
        let emitter = Arc::new(UnixEventEmitter::new("/nonexistent/producer.sock"));
        // Tamper flag set: the daemon detected a broken chain at startup.
        let server = Arc::new(IngestServer::new(
            Arc::clone(&ledger),
            emitter,
            Arc::new(AtomicBool::new(true)),
        ));

        let (mut client, server_end) = UnixStream::pair().unwrap();
        let serving = tokio::spawn(async move {
            let _ = server.serve_connection(server_end, "ai-agent").await;
        });

        let req = IngestRequest {
            kind: AuditKind::Query,
            structural: StructuralRecord {
                subject: "graph".into(),
                node_types: vec![],
                relations: vec![],
                result_count: None,
                duration_ms: None,
                outcome: "ok".into(),
                depth: None,
                capability_change: None,
            },
            forensic: None,
            call_chain_id: None,
            project_id: None,
        };
        write_frame(&mut client, &serde_json::to_vec(&req).unwrap())
            .await
            .unwrap();

        let reply = read_frame(&mut client).await.unwrap();
        match serde_json::from_slice::<IngestResponse>(&reply).unwrap() {
            IngestResponse::Unavailable { .. } => {}
            other => panic!("tampered ledger must refuse the append, got {other:?}"),
        }
        // Nothing was written: the ledger is still empty.
        assert_eq!(ledger.lock().await.verify().await.unwrap(), 0);

        drop(client);
        let _ = serving.await;
    }
}
