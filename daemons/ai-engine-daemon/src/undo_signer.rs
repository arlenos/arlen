//! A thin client to the separate-uid undo signer (`ai-undo-proto`): submit a
//! created undo entry to the signed, HMAC-chained log so a graph compensation
//! survives a restart.
//!
//! Best-effort from the executor's view: the in-memory compensation store is the
//! live undo mechanism for the current session, and a signer that is absent or
//! failing must never fail the write it would have recorded.

use std::path::Path;
use std::time::Duration;

use arlen_ai_undo_core::undo_log::UndoEntry;
use arlen_ai_undo_proto::{read_response, write_request, Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// A stuck signer (connection accepted, no reply) must never hang the executor's
/// write path; the whole submit is bounded by this.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Submit a created undo entry over an already-connected `stream`. Returns `Ok(())`
/// only when the signer sealed it; any transport, framing or non-`Sealed` reply is
/// an error the caller logs and swallows.
pub async fn submit_created_on<S>(stream: &mut S, entry: &UndoEntry) -> Result<(), String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::SubmitCreated(entry.clone()))
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::Sealed => Ok(()),
        other => Err(format!("signer did not seal the entry: {other:?}")),
    }
}

/// Connect to the signer at `socket` and submit a created undo entry, bounded by
/// [`SUBMIT_TIMEOUT`]. Best-effort: a connect/transport/timeout failure is returned
/// for the caller to swallow, so a stalled or unreachable signer never hangs or
/// fails the write.
pub async fn submit_created(socket: &Path, entry: &UndoEntry) -> Result<(), String> {
    let submit = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        submit_created_on(&mut stream, entry).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, submit).await {
        Ok(result) => result,
        Err(_) => Err(format!("signer submit timed out after {SUBMIT_TIMEOUT:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::InverseReceipt;
    use arlen_ai_undo_proto::{read_request, write_response};

    fn entry(op: &str) -> UndoEntry {
        UndoEntry {
            op_id: op.to_string(),
            correlation_id: op.to_string(),
            inverse: InverseReceipt::RetractGraphEdge {
                op_id: op.to_string(),
                from_type: "system.File".into(),
                from_id: "/x".into(),
                to_type: "system.Project".into(),
                to_id: "p".into(),
                relation_type: "FILE_PART_OF".into(),
            },
        }
    }

    #[tokio::test]
    async fn submit_seals_a_created_entry_via_a_signer() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let e = entry("op-1");
        let signer = tokio::spawn(async move {
            let req = read_request(&mut server).await.unwrap();
            match req {
                Request::SubmitCreated(got) => assert_eq!(got.op_id, "op-1"),
                other => panic!("expected SubmitCreated, got {other:?}"),
            }
            write_response(&mut server, &Response::Sealed).await.unwrap();
        });
        submit_created_on(&mut client, &e).await.expect("sealed");
        signer.await.unwrap();
    }

    #[tokio::test]
    async fn a_non_sealed_reply_is_an_error() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let e = entry("op-2");
        let signer = tokio::spawn(async move {
            let _ = read_request(&mut server).await.unwrap();
            write_response(&mut server, &Response::Error("nope".into())).await.unwrap();
        });
        let r = submit_created_on(&mut client, &e).await;
        signer.await.unwrap();
        assert!(r.is_err(), "a non-Sealed reply must be an error");
    }
}
