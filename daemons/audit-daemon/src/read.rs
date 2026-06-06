//! Read API socket.
//!
//! Serves the audit log's Structural tier to the user's own
//! processes — the Anomaly Detector and the Settings audit viewer.
//! The socket is mode 0600, so it is reachable only by the owning
//! user, which is exactly who foundation §8.4.7 permits to read the
//! Structural log; no per-app allowlist is needed for read access to
//! the user's own metadata.
//!
//! The Forensic tier is never served here: the response type
//! [`crate::ledger::StructuralView`] has no field that can hold
//! Forensic content, and the underlying query does not select it.

use std::path::Path;
use std::sync::Arc;

use tokio::net::UnixStream;

use audit_proto::{read_frame, write_frame, ReadRequest, ReadResponse};
// Re-export so `arlen_auditd::read::read_socket_path` stays the
// canonical path the binary and tests use; the type lives in
// `audit-proto` so readers (the Anomaly Detector) share it.
pub use audit_proto::read_socket_path;

use crate::error::{AuditError, Result};
use crate::ledger::LedgerReader;

/// The read API server.
pub struct ReadServer {
    reader: Arc<LedgerReader>,
    /// Shared with the daemon (and the ingest server): set when
    /// startup verification found the ledger tampered. Reported on
    /// every page so a reader learns tamper state over the reliable
    /// poll path, not only via a best-effort `audit.tampered` event.
    tampered: Arc<std::sync::atomic::AtomicBool>,
}

impl ReadServer {
    /// Build a server over a read-only ledger handle and the shared
    /// tamper flag.
    pub fn new(
        reader: Arc<LedgerReader>,
        tampered: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self { reader, tampered }
    }

    /// Bind the read socket and serve it until the accept loop
    /// errors. The daemon spawns this as a long-lived task.
    pub async fn run(self: Arc<Self>, socket_path: &Path) -> Result<()> {
        let listener = crate::bind_unix_socket(socket_path)?;
        tracing::info!(socket = %socket_path.display(), "audit read API listening");
        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|e| AuditError::Storage(format!("read accept: {e}")))?;
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = server.handle(stream).await {
                    tracing::warn!("read connection error: {e}");
                }
            });
        }
    }

    /// Handle one connection: field read queries until it closes.
    async fn handle(&self, mut stream: UnixStream) -> Result<()> {
        loop {
            let body = match read_frame(&mut stream).await {
                Ok(body) => body,
                // Closed connection or framing error ends the session.
                Err(_) => return Ok(()),
            };
            let response = self.serve(&body).await;
            let encoded = serde_json::to_vec(&response).map_err(|e| {
                AuditError::Storage(format!("encode read response: {e}"))
            })?;
            write_frame(&mut stream, &encoded).await?;
        }
    }

    /// Run one query and build the response.
    async fn serve(&self, body: &[u8]) -> ReadResponse {
        let req: ReadRequest = match serde_json::from_slice(body) {
            Ok(req) => req,
            Err(e) => {
                return ReadResponse::Error {
                    reason: format!("malformed read request: {e}"),
                }
            }
        };
        match self
            .reader
            .read_structural(req.from, req.to, req.limit, req.project_id.as_deref())
            .await
        {
            Ok(entries) => {
                // `head` reflects the SAME filter as the page (a
                // project-scoped read must not leak the global volume)
                // and is read after the page, so it is at least the
                // highest index returned. On a head-probe failure it
                // degrades to `last returned index + 1` rather than an
                // impossible 0 under a populated page, preserving the
                // invariant `head >= max(returned index) + 1`.
                let fallback_head = entries.last().map_or(0, |e| e.index + 1);
                let head = match self.reader.head(req.project_id.as_deref()).await {
                    Ok(h) => h.max(fallback_head),
                    Err(e) => {
                        tracing::warn!("read head probe failed: {e}");
                        fallback_head
                    }
                };
                ReadResponse::Page {
                    entries,
                    tampered: self.tampered.load(std::sync::atomic::Ordering::SeqCst),
                    head,
                }
            }
            Err(e) => ReadResponse::Error {
                reason: e.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{AuditKind, Ledger, StructuralRecord};

    fn structural(subject: &str) -> StructuralRecord {
        StructuralRecord {
            subject: subject.into(),
            node_types: vec![],
            relations: vec![],
            result_count: None,
            duration_ms: Some(1),
            outcome: "ok".into(),
            depth: None,
        }
    }

    #[test]
    fn read_socket_path_is_under_arlen() {
        let p = read_socket_path();
        assert!(
            p.to_string_lossy().ends_with("arlen/audit-read.sock"),
            "{}",
            p.display()
        );
    }

    /// Append entries with a writer, then read them back over the
    /// socket and confirm the page round-trips.
    #[tokio::test]
    async fn the_read_socket_returns_a_page_of_entries() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ledger.db");
        {
            let mut ledger = Ledger::open(&db, b"test-key".to_vec()).await.unwrap();
            for _ in 0..3 {
                ledger
                    .append(
                        AuditKind::Query,
                        "ai-daemon",
                        &structural("graph"),
                        None,
                        None,
                        None,
                    )
                    .await
                    .unwrap();
            }
        }

        let reader = Arc::new(LedgerReader::open(&db).await.unwrap());
        let tampered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = Arc::new(ReadServer::new(reader, tampered));
        let socket = dir.path().join("audit-read.sock");
        let socket_for_task = socket.clone();
        let serving = tokio::spawn(async move {
            let _ = server.run(&socket_for_task).await;
        });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut client = UnixStream::connect(&socket).await.unwrap();
        let req = ReadRequest {
            from: 0,
            to: u64::MAX,
            limit: 100,
            project_id: None,
        };
        write_frame(&mut client, &serde_json::to_vec(&req).unwrap())
            .await
            .unwrap();
        let reply = read_frame(&mut client).await.unwrap();
        let resp: ReadResponse = serde_json::from_slice(&reply).unwrap();
        match resp {
            ReadResponse::Page {
                entries,
                tampered,
                head,
            } => {
                assert_eq!(entries.len(), 3);
                assert_eq!(entries[0].index, 0);
                assert_eq!(entries[2].index, 2);
                assert_eq!(entries[0].actor, "ai-daemon");
                assert_eq!(entries[0].entry_hash_hex.len(), 64);
                assert!(!tampered, "untampered ledger reports tampered=false");
                assert_eq!(head, 3, "head is one past the highest index");
            }
            ReadResponse::Error { reason } => panic!("read failed: {reason}"),
        }

        serving.abort();
    }
}
