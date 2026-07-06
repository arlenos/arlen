//! The broker client: how Settings, the AI daemon + agent read and
//! mutate the AI master switches once they stop touching `ai.toml`
//! directly. Each call is a fresh connect (the state changes rarely
//! and a held connection buys nothing), so a transient broker outage
//! is a per-call error the caller handles - it never wedges.
//!
//! Reads ([`ConfigBrokerClient::get`]) are open to any same-uid
//! caller; a [`ConfigBrokerClient::set`] succeeds only if the daemon
//! resolves this process to an admitted writer, otherwise it returns
//! [`ClientError::Refused`]. The client never trusts a partial reply:
//! a broker `Error` (corrupt store) surfaces as [`ClientError::Broker`]
//! so the caller refuses rather than acting on a guess.

use std::path::PathBuf;

use thiserror::Error;
use tokio::net::UnixStream;

use crate::protocol::{read_frame_async, write_frame_async, Request, Response};
use crate::state::AiMasterSwitches;

/// A failure talking to the broker.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Connect/read/write failed (broker down, socket gone, framing).
    #[error("config-broker transport: {0}")]
    Transport(String),
    /// The broker refused a `Set` (the caller is not an admitted
    /// writer).
    #[error("config-broker refused: {0}")]
    Refused(String),
    /// The broker reported a store error (e.g. a corrupt state file);
    /// the caller must not proceed on a guessed state.
    #[error("config-broker error: {0}")]
    Broker(String),
    /// The broker returned a reply of the wrong shape for the request.
    #[error("config-broker unexpected reply")]
    Unexpected,
}

/// A thin client over the broker socket. Cheap to clone (just the
/// path); each request opens its own connection.
#[derive(Debug, Clone)]
pub struct ConfigBrokerClient {
    socket: PathBuf,
}

impl ConfigBrokerClient {
    /// A client for an explicit socket path.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// A client for the default broker socket
    /// ([`crate::server::socket_path`]).
    pub fn default_socket() -> Self {
        Self::new(crate::server::socket_path())
    }

    async fn connect(&self) -> Result<UnixStream, ClientError> {
        UnixStream::connect(&self.socket).await.map_err(|e| {
            ClientError::Transport(format!("connect {}: {e}", self.socket.display()))
        })
    }

    /// Read the current master switches.
    pub async fn get(&self) -> Result<AiMasterSwitches, ClientError> {
        let mut stream = self.connect().await?;
        write_frame_async(&mut stream, &Request::Get)
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let resp: Response = read_frame_async(&mut stream)
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        match resp {
            Response::State(s) => Ok(s),
            Response::Error(e) => Err(ClientError::Broker(e)),
            Response::Refused(r) => Err(ClientError::Refused(r)),
            Response::Committed => Err(ClientError::Unexpected),
        }
    }

    /// Replace the master switches. Returns [`ClientError::Refused`]
    /// if this process is not an admitted writer.
    pub async fn set(&self, switches: &AiMasterSwitches) -> Result<(), ClientError> {
        let mut stream = self.connect().await?;
        write_frame_async(&mut stream, &Request::Set(switches.clone()))
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let resp: Response = read_frame_async(&mut stream)
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        match resp {
            Response::Committed => Ok(()),
            Response::Refused(r) => Err(ClientError::Refused(r)),
            Response::Error(e) => Err(ClientError::Broker(e)),
            Response::State(_) => Err(ClientError::Unexpected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{bind_socket, current_uid, serve_connection};
    use crate::state::StateStore;
    use std::sync::Arc;

    /// Spawn an in-process broker over a temp socket + store, run one
    /// connection, and return (client, store, _tempdir guard).
    async fn spawn_broker() -> (ConfigBrokerClient, Arc<StateStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(dir.path()).unwrap());
        let sock = dir.path().join("broker.sock");
        let listener = bind_socket(&sock).unwrap();
        let uid = current_uid();
        let srv_store = Arc::clone(&store);
        tokio::spawn(async move {
            // Serve connections for the life of the test.
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let s = Arc::clone(&srv_store);
                        let sink = std::sync::Arc::new(audit_proto::sink::MockAuditSink::accepting());
                        tokio::spawn(async move { serve_connection(stream, s, uid, sink).await });
                    }
                    Err(_) => return,
                }
            }
        });
        (ConfigBrokerClient::new(sock), store, dir)
    }

    /// The client reads back the stored state over the real socket.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn client_get_reads_the_state() {
        let (client, store, _g) = spawn_broker().await;
        let want = AiMasterSwitches {
            enabled: true,
            access_level: 3,
            ..Default::default()
        };
        store.store(&want).unwrap();
        assert_eq!(client.get().await.unwrap(), want);
    }

    /// A `set` from the non-admitted test caller maps to `Refused`
    /// and leaves the store untouched.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn client_set_from_a_non_admitted_caller_is_refused() {
        let (client, store, _g) = spawn_broker().await;
        let hostile = AiMasterSwitches {
            executor_live: true,
            ..Default::default()
        };
        match client.set(&hostile).await {
            Err(ClientError::Refused(_)) => {}
            other => panic!("expected Refused, got {other:?}"),
        }
        assert_eq!(store.load().unwrap(), AiMasterSwitches::default());
    }

    /// A client pointed at a dead socket fails transport-closed, not
    /// by hanging or fabricating a state.
    #[tokio::test]
    async fn client_against_a_missing_broker_fails_transport() {
        let client = ConfigBrokerClient::new("/nonexistent/arlen/config-broker.sock");
        assert!(matches!(client.get().await, Err(ClientError::Transport(_))));
    }
}
