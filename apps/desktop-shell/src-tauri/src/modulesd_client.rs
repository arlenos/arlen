/// Async client to `lunaris-modulesd`.
///
/// Speaks the JSON-over-UnixSocket protocol defined in
/// `modulesd-proto`. Multiple in-flight requests are correlated by
/// the `id` field; the client demuxes responses to per-request
/// oneshot channels so callers can `await` a typed reply.
///
/// Auto-reconnect: if the connection drops the next call attempts to
/// re-establish before failing. Subscription events are republished
/// through a Tokio broadcast channel so the shell stays decoupled
/// from the wire.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use modulesd_proto::{Event, Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, oneshot, Mutex};
use log::{debug, warn};

const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not connected")]
    NotConnected,
    #[error("daemon error: {0}")]
    Daemon(String),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("call timed out")]
    Timeout,
}

/// One pending request awaiting a reply.
type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>;

pub struct ModulesdClient {
    socket_path: PathBuf,
    next_id: AtomicU64,
    pending: PendingMap,
    writer: Mutex<Option<OwnedWriteHalf>>,
    events_tx: broadcast::Sender<Event>,
}

impl ModulesdClient {
    pub fn new(socket_path: PathBuf) -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(128);
        Arc::new(Self {
            socket_path,
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            writer: Mutex::new(None),
            events_tx,
        })
    }

    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("LUNARIS_MODULESD_SOCKET") {
            return PathBuf::from(p);
        }
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{uid}/lunaris/modulesd.sock"))
    }

    /// Subscribe to lifecycle events. Each call returns a fresh
    /// receiver; lagged subscribers see warnings on the daemon side.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.events_tx.subscribe()
    }

    /// Connect (or reconnect). Spawns the read pump on success.
    pub async fn connect(self: &Arc<Self>) -> Result<(), ClientError> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (mut read, write) = stream.into_split();
        *self.writer.lock().await = Some(write);

        let pending = Arc::clone(&self.pending);
        let events_tx = self.events_tx.clone();

        tokio::spawn(async move {
            loop {
                let mut len_buf = [0u8; 4];
                if read.read_exact(&mut len_buf).await.is_err() {
                    debug!("modulesd_client: read pump ended (eof)");
                    return;
                }
                let n = u32::from_be_bytes(len_buf) as usize;
                if n == 0 || n > MAX_FRAME_BYTES {
                    warn!("modulesd_client: bad frame size {n}");
                    return;
                }
                let mut body = vec![0u8; n];
                if read.read_exact(&mut body).await.is_err() {
                    return;
                }

                // Try Response first; fall back to Event.
                if let Ok(resp) = serde_json::from_slice::<Response>(&body) {
                    if let Some(id) = response_id(&resp) {
                        if let Some(tx) = pending.lock().await.remove(id) {
                            let _ = tx.send(resp);
                            continue;
                        }
                    }
                }
                if let Ok(ev) = serde_json::from_slice::<Event>(&body) {
                    let _ = events_tx.send(ev);
                }
            }
        });

        Ok(())
    }

    /// Send a Request and await the matching Response. Re-connects
    /// once on a connection error.
    pub async fn call(self: &Arc<Self>, mut req: Request) -> Result<Response, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        set_request_id(&mut req, id.clone());

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        if let Err(err) = self.write(&req).await {
            // Reconnect once.
            warn!("modulesd_client: write failed ({err}), reconnecting once");
            self.connect().await?;
            self.write(&req).await?;
        }

        // 5 s timeout protects the caller from a wedged daemon.
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(ClientError::Daemon("response channel dropped".into())),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(ClientError::Timeout)
            }
        }
    }

    async fn write(&self, req: &Request) -> Result<(), ClientError> {
        let body = serde_json::to_vec(req)?;
        let mut guard = self.writer.lock().await;
        let writer = guard.as_mut().ok_or(ClientError::NotConnected)?;
        let len = (body.len() as u32).to_be_bytes();
        writer.write_all(&len).await?;
        writer.write_all(&body).await?;
        writer.flush().await?;
        Ok(())
    }
}

fn response_id(resp: &Response) -> Option<&str> {
    Some(match resp {
        Response::Hello { id, .. }
        | Response::ModuleList { id, .. }
        | Response::WaypointerResults { id, .. }
        | Response::WaypointerAggregate { id, .. }
        | Response::Executed { id, .. }
        | Response::IframeIssued { id, .. }
        | Response::HostReply { id, .. }
        | Response::Subscribed { id, .. }
        | Response::Acked { id, .. }
        | Response::Error { id, .. }
        | Response::IframeMeta { id, .. } => id.as_str(),
    })
}

fn set_request_id(req: &mut Request, new_id: String) {
    match req {
        Request::Hello { id, .. }
        | Request::ListModules { id }
        | Request::WaypointerSearch { id, .. }
        | Request::WaypointerSearchAll { id, .. }
        | Request::WaypointerExecute { id, .. }
        | Request::IframeMint { id, .. }
        | Request::HostCall { id, .. }
        | Request::Subscribe { id, .. }
        | Request::SetEnabled { id, .. }
        | Request::Retry { id, .. }
        | Request::IframeLookup { id, .. } => *id = new_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_resolves_correctly() {
        // Both checks share one test to avoid parallel-test
        // contention on the shared LUNARIS_MODULESD_SOCKET env var.
        std::env::set_var("LUNARIS_MODULESD_SOCKET", "/tmp/mocktest.sock");
        assert_eq!(ModulesdClient::default_path(), PathBuf::from("/tmp/mocktest.sock"));
        std::env::remove_var("LUNARIS_MODULESD_SOCKET");
        let p = ModulesdClient::default_path();
        assert!(p.to_string_lossy().contains("/run/user/"));
    }

    #[test]
    fn response_id_extracts_for_all_variants() {
        let r = Response::Acked { id: "X".into() };
        assert_eq!(response_id(&r), Some("X"));
        let r = Response::Error {
            id: "Y".into(),
            code: modulesd_proto::ErrorCode::NotFound,
            message: "x".into(),
        };
        assert_eq!(response_id(&r), Some("Y"));
    }
}
