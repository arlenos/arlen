//! A thin client to the separate-uid undo signer (`ai-undo-proto`): submit a
//! created undo entry to the signed, HMAC-chained log so a graph compensation
//! survives a restart.
//!
//! Best-effort from the executor's view: the in-memory compensation store is the
//! live undo mechanism for the current session, and a signer that is absent or
//! failing must never fail the write it would have recorded.

use std::path::Path;
use std::time::Duration;

use arlen_ai_undo_core::undo_log::{UndoEntry, UndoState};
use arlen_ai_undo_proto::{read_response, write_request, RecentEntry, Request, Response, StateReply};
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

/// Fetch the signer's live (non-terminal) entries over an already-connected
/// `stream`. Returns the sealed entries a restarting consumer re-arms; any
/// transport, framing or non-`Entries` reply is an error the caller logs and
/// swallows (best-effort: an absent signer just means no restore).
pub async fn fetch_live_on<S>(stream: &mut S) -> Result<Vec<UndoEntry>, String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::LiveEntries)
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::Entries(entries) => Ok(entries),
        other => Err(format!("signer did not return live entries: {other:?}")),
    }
}

/// Fetch only the entries a crash caught mid-reversal, over an open stream.
pub async fn fetch_compensating_on<S>(stream: &mut S) -> Result<Vec<UndoEntry>, String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::CompensatingEntries)
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::Entries(entries) => Ok(entries),
        other => Err(format!("signer did not return compensating entries: {other:?}")),
    }
}

/// Connect to the signer at `socket` and fetch the entries a crash caught
/// mid-reversal, bounded by [`SUBMIT_TIMEOUT`]. Best-effort like
/// [`fetch_live`]: an unreachable signer leaves the interrupted reversals for
/// the next restart rather than failing startup.
pub async fn fetch_compensating(socket: &Path) -> Result<Vec<UndoEntry>, String> {
    let fetch = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        fetch_compensating_on(&mut stream).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, fetch).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "signer compensating-entries fetch timed out after {SUBMIT_TIMEOUT:?}"
        )),
    }
}

/// Connect to the signer at `socket` and fetch its live entries, bounded by
/// [`SUBMIT_TIMEOUT`]. Best-effort: a connect/transport/timeout failure is
/// returned for the caller to swallow, so an unreachable signer never hangs or
/// fails startup - the session simply starts with no persisted undo restored.
pub async fn fetch_live(socket: &Path) -> Result<Vec<UndoEntry>, String> {
    let fetch = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        fetch_live_on(&mut stream).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, fetch).await {
        Ok(result) => result,
        Err(_) => Err(format!("signer live-entries fetch timed out after {SUBMIT_TIMEOUT:?}")),
    }
}

/// Fetch the most recent entries with their folded state, newest first, over an
/// already-connected `stream`. This is the read a recent-actions surface makes:
/// unlike [`fetch_live_on`] it keeps terminal entries, because a completed undo
/// is the evidence the user came to look for.
pub async fn fetch_recent_on<S>(stream: &mut S, limit: u32) -> Result<Vec<RecentEntry>, String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::ListRecent { limit })
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::Recent(entries) => Ok(entries),
        other => Err(format!("signer did not return recent entries: {other:?}")),
    }
}

/// Connect to the signer at `socket` and fetch its recent entries, bounded by
/// [`SUBMIT_TIMEOUT`]. Best-effort like the other reads: an unreachable signer
/// leaves the surface empty rather than failing it.
pub async fn fetch_recent(socket: &Path, limit: u32) -> Result<Vec<RecentEntry>, String> {
    let fetch = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        fetch_recent_on(&mut stream, limit).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, fetch).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "signer recent-entries fetch timed out after {SUBMIT_TIMEOUT:?}"
        )),
    }
}

/// Look up one entry by its op id over an already-connected `stream`. Returns the
/// sealed [`UndoEntry`] (its captured inverse), `None` if the signer has no such
/// entry, or an error on any transport/framing/non-`Entry` reply. This is the read
/// the live undo path uses to recover a NON-GRAPH inverse (a filesystem/settings
/// receipt the in-memory graph compensation store never held) so it can be enacted.
pub async fn lookup_entry_on<S>(stream: &mut S, op_id: &str) -> Result<Option<UndoEntry>, String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::LookupEntry { op_id: op_id.to_string() })
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::Entry(entry) => Ok(entry),
        other => Err(format!("signer did not return an entry: {other:?}")),
    }
}

/// Connect to the signer at `socket` and look up an entry by op id, bounded by
/// [`SUBMIT_TIMEOUT`]. A connect/transport/timeout failure is returned for the
/// caller to handle fail-closed (an unreachable signer must not let an undo proceed
/// on stale assumptions); a successful lookup returns the entry or `None`.
pub async fn lookup_entry(socket: &Path, op_id: &str) -> Result<Option<UndoEntry>, String> {
    let lookup = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        lookup_entry_on(&mut stream, op_id).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, lookup).await {
        Ok(result) => result,
        Err(_) => Err(format!("signer lookup timed out after {SUBMIT_TIMEOUT:?}")),
    }
}

/// Record a lifecycle transition for an entry over an already-connected `stream`.
/// Used to mark a non-graph inverse `Compensated`/`Superseded` after a live undo has
/// enacted it, so a second undo of the same id is a no-op rather than a re-enact. A
/// non-`Sealed` reply (the signer rejected an illegal transition, or a transport
/// failure) is an error the caller handles.
pub async fn transition_on<S>(stream: &mut S, op_id: &str, state: UndoState) -> Result<(), String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::Transition { op_id: op_id.to_string(), state })
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::Sealed => Ok(()),
        other => Err(format!("signer did not seal the transition: {other:?}")),
    }
}

/// Connect to the signer at `socket` and record a lifecycle transition, bounded by
/// [`SUBMIT_TIMEOUT`]. Best-effort at the call site: a transition that cannot be
/// recorded (an unreachable signer, an illegal transition) is returned for the caller
/// to log, since the undo it marks has already happened.
pub async fn transition(socket: &Path, op_id: &str, state: UndoState) -> Result<(), String> {
    let go = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        transition_on(&mut stream, op_id, state).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, go).await {
        Ok(result) => result,
        Err(_) => Err(format!("signer transition timed out after {SUBMIT_TIMEOUT:?}")),
    }
}

/// Look up an entry's current folded lifecycle state over an already-connected
/// `stream`. Returns the [`StateReply`] (`Absent` / `Present(state)` / `Corrupt`), so
/// the live undo path can skip an already-terminal entry (idempotency) and refuse a
/// corrupt one fail-closed.
pub async fn lookup_state_on<S>(stream: &mut S, op_id: &str) -> Result<StateReply, String>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    write_request(stream, &Request::LookupState { op_id: op_id.to_string() })
        .await
        .map_err(|e| format!("write: {e}"))?;
    match read_response(stream)
        .await
        .map_err(|e| format!("read: {e}"))?
    {
        Response::State(reply) => Ok(reply),
        other => Err(format!("signer did not return a state: {other:?}")),
    }
}

/// Connect to the signer at `socket` and look up an entry's lifecycle state, bounded
/// by [`SUBMIT_TIMEOUT`]. A connect/transport/timeout failure is returned for the
/// caller to handle fail-closed.
pub async fn lookup_state(socket: &Path, op_id: &str) -> Result<StateReply, String> {
    let go = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect {}: {e}", socket.display()))?;
        lookup_state_on(&mut stream, op_id).await
    };
    match tokio::time::timeout(SUBMIT_TIMEOUT, go).await {
        Ok(result) => result,
        Err(_) => Err(format!("signer state lookup timed out after {SUBMIT_TIMEOUT:?}")),
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

    #[tokio::test]
    async fn fetch_live_returns_the_signers_entries() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let e = entry("op-live");
        let served = e.clone();
        let signer = tokio::spawn(async move {
            match read_request(&mut server).await.unwrap() {
                Request::LiveEntries => {}
                other => panic!("expected LiveEntries, got {other:?}"),
            }
            write_response(&mut server, &Response::Entries(vec![served])).await.unwrap();
        });
        let got = fetch_live_on(&mut client).await.expect("entries");
        signer.await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].op_id, "op-live");
    }

    #[tokio::test]
    async fn a_non_entries_reply_to_fetch_live_is_an_error() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let signer = tokio::spawn(async move {
            let _ = read_request(&mut server).await.unwrap();
            write_response(&mut server, &Response::Error("no".into())).await.unwrap();
        });
        let r = fetch_live_on(&mut client).await;
        signer.await.unwrap();
        assert!(r.is_err(), "a non-Entries reply must be an error");
    }

    #[tokio::test]
    async fn lookup_entry_returns_the_entry_by_op_id() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let served = entry("op-look");
        let expected = served.clone();
        let signer = tokio::spawn(async move {
            match read_request(&mut server).await.unwrap() {
                Request::LookupEntry { op_id } => assert_eq!(op_id, "op-look"),
                other => panic!("expected LookupEntry, got {other:?}"),
            }
            write_response(&mut server, &Response::Entry(Some(served))).await.unwrap();
        });
        let got = lookup_entry_on(&mut client, "op-look").await.expect("entry");
        signer.await.unwrap();
        assert_eq!(got.map(|e| e.op_id), Some(expected.op_id));
    }

    #[tokio::test]
    async fn lookup_entry_maps_an_absent_entry_to_none() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let signer = tokio::spawn(async move {
            let _ = read_request(&mut server).await.unwrap();
            write_response(&mut server, &Response::Entry(None)).await.unwrap();
        });
        let got = lookup_entry_on(&mut client, "missing").await.expect("ok");
        signer.await.unwrap();
        assert!(got.is_none(), "an absent entry maps to None, not an error");
    }

    #[tokio::test]
    async fn a_non_entry_reply_to_lookup_is_an_error() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let signer = tokio::spawn(async move {
            let _ = read_request(&mut server).await.unwrap();
            write_response(&mut server, &Response::Error("no".into())).await.unwrap();
        });
        let r = lookup_entry_on(&mut client, "x").await;
        signer.await.unwrap();
        assert!(r.is_err(), "a non-Entry reply must be an error");
    }

    #[tokio::test]
    async fn transition_seals_a_recorded_state_change() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let signer = tokio::spawn(async move {
            match read_request(&mut server).await.unwrap() {
                Request::Transition { op_id, state } => {
                    assert_eq!(op_id, "op-t");
                    assert_eq!(state, UndoState::Superseded);
                }
                other => panic!("expected Transition, got {other:?}"),
            }
            write_response(&mut server, &Response::Sealed).await.unwrap();
        });
        transition_on(&mut client, "op-t", UndoState::Superseded).await.expect("sealed");
        signer.await.unwrap();
    }

    #[tokio::test]
    async fn a_rejected_transition_is_an_error() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let signer = tokio::spawn(async move {
            let _ = read_request(&mut server).await.unwrap();
            write_response(&mut server, &Response::Error("illegal".into())).await.unwrap();
        });
        let r = transition_on(&mut client, "op-t", UndoState::Compensated).await;
        signer.await.unwrap();
        assert!(r.is_err(), "a rejected transition must be an error");
    }

    #[tokio::test]
    async fn lookup_state_returns_the_folded_state() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let signer = tokio::spawn(async move {
            match read_request(&mut server).await.unwrap() {
                Request::LookupState { op_id } => assert_eq!(op_id, "op-s"),
                other => panic!("expected LookupState, got {other:?}"),
            }
            write_response(&mut server, &Response::State(StateReply::Present(UndoState::Superseded)))
                .await
                .unwrap();
        });
        let reply = lookup_state_on(&mut client, "op-s").await.expect("state");
        signer.await.unwrap();
        assert_eq!(reply, StateReply::Present(UndoState::Superseded));
    }
}

