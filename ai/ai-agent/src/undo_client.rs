//! The agent-side client to the undo-log signer
//! (reversible-receipts-and-the-effect-model.md EM-R1).
//!
//! The executor submits an inverse-receipt entry write-ahead before an
//! externalised non-graph act, records lifecycle transitions as the act commits
//! or is compensated, and looks the durable state back when reconciling. This is
//! the thin transport: it connects to the signer's rendezvous socket and maps
//! each [`arlen_ai_undo_proto::Request`]/`Response` pair to a typed result. The
//! signer owns the key and the chained log; the agent reaches them only through
//! this socket. As built the signer runs same-uid, so the separation is
//! defense-in-depth, not integrity against a fully-compromised agent; the
//! separate-uid property that would withhold the key from such an agent is
//! provisioned at deploy time and closed by the installd inode registry plus a
//! TPM-sealed counter, per the signer's `auth.rs`.

use std::path::Path;

use arlen_ai_undo_core::undo_log::{UndoEntry, UndoState};
use arlen_ai_undo_proto::{
    read_response, socket_path, write_request, ProtoError, Request, Response, StateReply,
};
use tokio::net::UnixStream;

/// An error talking to the signer.
#[derive(Debug, thiserror::Error)]
pub enum SignerClientError {
    /// A framing, codec, or I/O failure on the connection (including connect).
    #[error("undo-signer transport error: {0}")]
    Transport(#[from] ProtoError),
    /// The signer accepted the request but refused it (an illegal submission, an
    /// integrity failure). The string is the signer's coarse reason.
    #[error("undo-signer rejected the request: {0}")]
    Rejected(String),
    /// The signer returned a response of the wrong kind for the request (a
    /// protocol violation; never expected from the real signer).
    #[error("undo-signer returned an unexpected response: {0}")]
    Unexpected(String),
}

/// A connection to the signer. Holds one open socket; the signer's serve loop
/// fields many requests on it. On a transport error the caller drops and
/// reconnects (the signer is connectionless beyond the stream).
#[derive(Debug)]
pub struct SignerClient {
    stream: UnixStream,
}

impl SignerClient {
    /// Connect to the signer at the default rendezvous path.
    pub async fn connect_default() -> Result<Self, SignerClientError> {
        Self::connect(&socket_path()).await
    }

    /// Connect to the signer at `path`.
    pub async fn connect(path: &Path) -> Result<Self, SignerClientError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| SignerClientError::Transport(ProtoError::Io(e)))?;
        Ok(Self { stream })
    }

    /// Wrap an already-connected stream (for injection in tests).
    pub fn from_stream(stream: UnixStream) -> Self {
        Self { stream }
    }

    async fn round_trip(&mut self, request: Request) -> Result<Response, SignerClientError> {
        write_request(&mut self.stream, &request).await?;
        Ok(read_response(&mut self.stream).await?)
    }

    /// Submit a newly-created entry write-ahead (its lifecycle begins `InFlight`).
    pub async fn submit_created(&mut self, entry: UndoEntry) -> Result<(), SignerClientError> {
        match self.round_trip(Request::SubmitCreated(entry)).await? {
            Response::Sealed => Ok(()),
            Response::Error(m) => Err(SignerClientError::Rejected(m)),
            other => Err(SignerClientError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Record a lifecycle transition for an existing entry.
    pub async fn transition(
        &mut self,
        op_id: &str,
        state: UndoState,
    ) -> Result<(), SignerClientError> {
        let request = Request::Transition {
            op_id: op_id.to_string(),
            state,
        };
        match self.round_trip(request).await? {
            Response::Sealed => Ok(()),
            Response::Error(m) => Err(SignerClientError::Rejected(m)),
            other => Err(SignerClientError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Look up the current folded state of an entry.
    pub async fn lookup_state(&mut self, op_id: &str) -> Result<StateReply, SignerClientError> {
        let request = Request::LookupState {
            op_id: op_id.to_string(),
        };
        match self.round_trip(request).await? {
            Response::State(reply) => Ok(reply),
            Response::Error(m) => Err(SignerClientError::Rejected(m)),
            other => Err(SignerClientError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Look up the immutable created data of an entry (the captured inverse).
    pub async fn lookup_entry(
        &mut self,
        op_id: &str,
    ) -> Result<Option<UndoEntry>, SignerClientError> {
        let request = Request::LookupEntry {
            op_id: op_id.to_string(),
        };
        match self.round_trip(request).await? {
            Response::Entry(entry) => Ok(entry),
            Response::Error(m) => Err(SignerClientError::Rejected(m)),
            other => Err(SignerClientError::Unexpected(format!("{other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};
    use arlen_ai_undo_proto::{read_request, write_response};

    fn entry(op: &str) -> UndoEntry {
        UndoEntry {
            op_id: op.to_string(),
            correlation_id: "run".to_string(),
            inverse: InverseReceipt::RestorePath {
                now: CanonicalPath::new("/b/x").unwrap(),
                prior: CanonicalPath::new("/a/x").unwrap(),
            },
        }
    }

    // A minimal mock signer on the server end of a socket pair: it reads each
    // request, asserts/echoes via a canned response. Tests the client's request
    // encoding and response decoding against the real wire protocol, without
    // depending on the signer crate.
    #[tokio::test]
    async fn client_submits_and_reads_back_over_the_wire() {
        let (client_end, mut server_end) = UnixStream::pair().unwrap();
        let server = tokio::spawn(async move {
            // 1: a SubmitCreated → Sealed.
            match read_request(&mut server_end).await.unwrap() {
                Request::SubmitCreated(e) => assert_eq!(e.op_id, "op-1"),
                other => panic!("expected SubmitCreated, got {other:?}"),
            }
            write_response(&mut server_end, &Response::Sealed).await.unwrap();
            // 2: a Transition → Sealed.
            match read_request(&mut server_end).await.unwrap() {
                Request::Transition { op_id, state } => {
                    assert_eq!(op_id, "op-1");
                    assert_eq!(state, UndoState::Committed);
                }
                other => panic!("expected Transition, got {other:?}"),
            }
            write_response(&mut server_end, &Response::Sealed).await.unwrap();
            // 3: a LookupState → Present(Committed).
            match read_request(&mut server_end).await.unwrap() {
                Request::LookupState { op_id } => assert_eq!(op_id, "op-1"),
                other => panic!("expected LookupState, got {other:?}"),
            }
            write_response(
                &mut server_end,
                &Response::State(StateReply::Present(UndoState::Committed)),
            )
            .await
            .unwrap();
            // 4: a LookupEntry → the entry.
            let _ = read_request(&mut server_end).await.unwrap();
            write_response(&mut server_end, &Response::Entry(Some(entry("op-1"))))
                .await
                .unwrap();
        });

        let mut client = SignerClient::from_stream(client_end);
        client.submit_created(entry("op-1")).await.unwrap();
        client.transition("op-1", UndoState::Committed).await.unwrap();
        assert_eq!(
            client.lookup_state("op-1").await.unwrap(),
            StateReply::Present(UndoState::Committed)
        );
        assert_eq!(client.lookup_entry("op-1").await.unwrap().unwrap().op_id, "op-1");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_error_response_maps_to_rejected() {
        let (client_end, mut server_end) = UnixStream::pair().unwrap();
        let server = tokio::spawn(async move {
            let _ = read_request(&mut server_end).await.unwrap();
            write_response(&mut server_end, &Response::Error("denied".into()))
                .await
                .unwrap();
        });
        let mut client = SignerClient::from_stream(client_end);
        match client.submit_created(entry("op-1")).await {
            Err(SignerClientError::Rejected(m)) => assert_eq!(m, "denied"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_closed_connection_is_a_transport_error() {
        let (client_end, server_end) = UnixStream::pair().unwrap();
        drop(server_end); // server gone before any response
        let mut client = SignerClient::from_stream(client_end);
        assert!(matches!(
            client.submit_created(entry("op-1")).await,
            Err(SignerClientError::Transport(_))
        ));
    }
}
