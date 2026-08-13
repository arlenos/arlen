//! Async client to `arlen-modulesd`.
//!
//! Speaks the JSON-over-UnixSocket protocol defined in
//! `modulesd-proto`. Multiple in-flight requests are correlated by
//! the `id` field; the client demuxes responses to per-request
//! oneshot channels so callers can `await` a typed reply.
//!
//! Auto-reconnect: if the connection drops the next call attempts to
//! re-establish before failing. Subscription events are republished
//! through a Tokio broadcast channel so the shell stays decoupled
//! from the wire.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use modulesd_proto::{Event, Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, oneshot, Mutex};
use log::{debug, info, warn};

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

/// How long to stay quiet after a failed connect before trying again.
///
/// The daemon may legitimately be absent - no third-party modules, or an image
/// that does not ship it - and in that state every call used to attempt a fresh
/// connect and log a warning. On the boot of 13 Aug that is what the log showed:
/// a component dialling a socket nothing serves, forever, which is noise standing
/// exactly where a real failure would need to be visible.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

pub struct ModulesdClient {
    socket_path: PathBuf,
    next_id: AtomicU64,
    pending: PendingMap,
    writer: Mutex<Option<OwnedWriteHalf>>,
    events_tx: broadcast::Sender<Event>,
    /// When the last connect attempt failed, so the next ones can be skipped
    /// rather than retried per call. `None` means nothing has failed since the
    /// last success - which is also the state a successful connect restores, so
    /// a daemon that appears later is reported as arriving.
    last_failure: Mutex<Option<std::time::Instant>>,
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
            last_failure: Mutex::new(None),
        })
    }

    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("ARLEN_MODULESD_SOCKET") {
            return PathBuf::from(p);
        }
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{uid}/arlen/modulesd.sock"))
    }

    /// Subscribe to lifecycle events. Each call returns a fresh
    /// receiver; lagged subscribers see warnings on the daemon side.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.events_tx.subscribe()
    }

    /// Connect (or reconnect). Spawns the read pump on success.
    ///
    /// Says an unreachable daemon ONCE and then goes quiet for [`RETRY_AFTER`],
    /// rather than logging per call. A caller still gets the error every time -
    /// the backoff hides the noise, never the failure.
    pub async fn connect(self: &Arc<Self>) -> Result<(), ClientError> {
        if let Some(since) = *self.last_failure.lock().await {
            if since.elapsed() < RETRY_AFTER {
                return Err(ClientError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    format!(
                        "modulesd is not reachable at {} (quiet until {}s after the \
                         first failure)",
                        self.socket_path.display(),
                        RETRY_AFTER.as_secs()
                    ),
                )));
            }
        }
        let stream = match UnixStream::connect(&self.socket_path).await {
            Ok(s) => s,
            Err(e) => {
                let mut failed = self.last_failure.lock().await;
                if failed.is_none() {
                    warn!(
                        "modulesd_client: {} is not reachable ({e}); modules stay \
                         unavailable and this will be retried every {}s without \
                         logging again",
                        self.socket_path.display(),
                        RETRY_AFTER.as_secs()
                    );
                }
                *failed = Some(std::time::Instant::now());
                return Err(e.into());
            }
        };
        if self.last_failure.lock().await.take().is_some() {
            info!("modulesd_client: {} is reachable again", self.socket_path.display());
        }
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
            // Reconnect once. At debug: with the daemon absent this fires on every
            // call, and the reason is already stated once by `connect`.
            debug!("modulesd_client: write failed ({err}), reconnecting once");
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
        // contention on the shared ARLEN_MODULESD_SOCKET env var.
        std::env::set_var("ARLEN_MODULESD_SOCKET", "/tmp/mocktest.sock");
        assert_eq!(ModulesdClient::default_path(), PathBuf::from("/tmp/mocktest.sock"));
        std::env::remove_var("ARLEN_MODULESD_SOCKET");
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

    #[tokio::test]
    async fn an_absent_daemon_is_refused_without_a_second_connect_attempt() {
        // The property the log needs: a caller still gets an error every time, and
        // the socket is only DIALLED once per backoff window. Checked by pointing
        // at a path nothing serves and asserting the second failure comes back as
        // the backoff refusal rather than a fresh connect error - same outcome for
        // the caller, no second syscall and no second log line.
        let dir = tempfile::tempdir().unwrap();
        let client = ModulesdClient::new(dir.path().join("absent.sock"));

        let first = client.connect().await.unwrap_err();
        assert!(
            matches!(&first, ClientError::Io(e) if e.kind() == std::io::ErrorKind::NotFound),
            "the first attempt really dials and really fails: {first:?}"
        );

        let second = client.connect().await.unwrap_err();
        match &second {
            ClientError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotConnected);
                assert!(
                    e.to_string().contains("not reachable"),
                    "the refusal says why rather than repeating the dial: {e}"
                );
            }
            other => panic!("expected the backoff refusal, got {other:?}"),
        }
    }
}
