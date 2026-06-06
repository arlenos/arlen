/// Unix-socket server.
///
/// Accepts connections at `/run/user/{uid}/arlen/modulesd.sock`,
/// frames JSON requests as `[u32 BE length][body]`, dispatches each
/// request to the manager, and broadcasts events to subscribed
/// connections.
///
/// Multi-client capable: each connection runs in its own task with a
/// per-connection writer half. The manager owns the source of truth;
/// the server is a thin transport layer.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, error, info, warn};

use crate::error::{DaemonError, Result};
use crate::manager::Manager;
use crate::socket::protocol::{Event, Request, Response};

const MAX_FRAME_BYTES: usize = 1024 * 1024;

pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_MODULESD_SOCKET") {
        return PathBuf::from(p);
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/arlen/modulesd.sock"))
}

pub struct SocketServer {
    listener: UnixListener,
    manager: Arc<Manager>,
    events_tx: broadcast::Sender<Event>,
}

impl SocketServer {
    /// Bind a fresh listener at `socket_path`, removing any stale
    /// socket file from a previous run first. Used when modulesd is
    /// started directly (not via systemd socket activation).
    pub fn bind(
        socket_path: &std::path::Path,
        manager: Arc<Manager>,
        events_tx: broadcast::Sender<Event>,
    ) -> Result<Self> {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        // Replace any stale socket from a previous run.
        let _ = std::fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path)?;
        info!("modulesd: listening on {}", socket_path.display());
        Ok(Self {
            listener,
            manager,
            events_tx,
        })
    }

    /// S7.6: inherit a listening socket from systemd socket
    /// activation when `LISTEN_FDS` is set, otherwise bind a fresh
    /// listener at `socket_path`. Avoids the race where systemd
    /// holds the listening socket and modulesd tries to create a
    /// second one at the same path (`EADDRINUSE`).
    ///
    /// systemd's socket activation protocol: file descriptors start
    /// at SD_LISTEN_FDS_START = 3 and there are `LISTEN_FDS` of
    /// them. modulesd's unit only declares one socket, so we take
    /// the first.
    pub fn bind_or_inherit(
        socket_path: &std::path::Path,
        manager: Arc<Manager>,
        events_tx: broadcast::Sender<Event>,
    ) -> Result<Self> {
        // sd-notify 0.4's `listen_fds()` returns an iterator over the
        // passed FDs. modulesd's unit declares exactly one socket, so
        // the first FD is our listener. On non-systemd runs (cargo
        // run, dev mode, custom init) `listen_fds` returns an empty
        // iterator (LISTEN_FDS unset).
        if let Ok(mut fds) = sd_notify::listen_fds() {
            if let Some(raw_fd) = fds.next() {
                // Take ownership of the FD so Drop on the std listener
                // closes it instead of leaking on shutdown.
                use std::os::unix::io::FromRawFd;
                let std_listener =
                    unsafe { std::os::unix::net::UnixListener::from_raw_fd(raw_fd) };
                std_listener.set_nonblocking(true)?;
                let listener = UnixListener::from_std(std_listener)
                    .map_err(DaemonError::Io)?;
                info!("modulesd: inherited socket activation listener (fd {raw_fd})");
                return Ok(Self {
                    listener,
                    manager,
                    events_tx,
                });
            }
        }
        Self::bind(socket_path, manager, events_tx)
    }

    pub async fn run(self) -> Result<()> {
        loop {
            match self.listener.accept().await {
                Ok((stream, _addr)) => {
                    let manager = Arc::clone(&self.manager);
                    let events_rx = self.events_tx.subscribe();
                    tokio::spawn(async move {
                        if let Err(err) = handle_connection(stream, manager, events_rx).await {
                            warn!("modulesd: connection ended with error: {err}");
                        }
                    });
                }
                Err(err) => {
                    error!("modulesd: accept failed: {err}");
                    // Brief sleep so a runaway accept loop does not
                    // saturate the CPU on a permanent failure.
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    manager: Arc<Manager>,
    mut events_rx: broadcast::Receiver<Event>,
) -> Result<()> {
    let (mut read, write) = stream.into_split();
    let writer = Arc::new(Mutex::new(write));

    // Spawn a per-connection event pump so unsolicited events go out
    // without blocking the request reader.
    let writer_for_events = Arc::clone(&writer);
    tokio::spawn(async move {
        loop {
            match events_rx.recv().await {
                Ok(ev) => {
                    if let Err(err) = write_event(&writer_for_events, &ev).await {
                        debug!("modulesd: event writer ending: {err}");
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("modulesd: subscriber lagged, dropped {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    loop {
        let mut len_buf = [0u8; 4];
        match read.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => return Err(err.into()),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return Err(DaemonError::Internal(format!("frame size out of range: {len}")));
        }
        let mut body = vec![0u8; len];
        read.read_exact(&mut body).await?;

        let request: Request = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(err) => {
                let resp = Response::Error {
                    id: String::new(),
                    code: crate::socket::protocol::ErrorCode::InvalidRequest,
                    message: err.to_string(),
                };
                write_response(&writer, &resp).await?;
                continue;
            }
        };

        let response = manager.handle_request(request).await;
        write_response(&writer, &response).await?;
    }
}

async fn write_response(writer: &Mutex<tokio::net::unix::OwnedWriteHalf>, resp: &Response) -> Result<()> {
    let bytes = serde_json::to_vec(resp)?;
    write_frame(writer, &bytes).await
}

async fn write_event(writer: &Mutex<tokio::net::unix::OwnedWriteHalf>, ev: &Event) -> Result<()> {
    // Frame events with the same `Event` envelope tag they would have
    // on disk. Subscribers parse the JSON and dispatch by `type`.
    let bytes = serde_json::to_vec(ev)?;
    write_frame(writer, &bytes).await
}

async fn write_frame(writer: &Mutex<tokio::net::unix::OwnedWriteHalf>, body: &[u8]) -> Result<()> {
    let mut w = writer.lock().await;
    let len = (body.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_path_resolves_correctly() {
        // One serial test to avoid parallel contention on the shared
        // LUNARIS_MODULESD_SOCKET env var.
        std::env::set_var("LUNARIS_MODULESD_SOCKET", "/tmp/test.sock");
        assert_eq!(default_socket_path(), PathBuf::from("/tmp/test.sock"));
        std::env::remove_var("LUNARIS_MODULESD_SOCKET");
        let p = default_socket_path();
        let s = p.to_string_lossy();
        assert!(s.contains("/run/user/"));
        assert!(s.ends_with("/arlen/modulesd.sock"));
    }
}
