//! The bottle daemon's Unix-socket server.
//!
//! WHY A DAEMON AND NOT A LIBRARY the Settings backend links: a launched Windows
//! program has to outlive the window that started it. A bottle whose supervising
//! process is the panel dies when somebody closes the panel, which is the one
//! thing a "runtime" must not do (`windows-apps-plan.md`).
//!
//! Per connection: authenticate the peer (`SO_PEERPIDFD` + uid), then field
//! requests until the peer closes or stops being alive. An auth failure drops the
//! connection without a word - a credential lookup that did not cleanly succeed
//! never serves.
//!
//! SAME-UID READS, no allowlist. The vocabulary here is read-only and the bottles
//! are the person's own; the config broker draws the same line, restricting only
//! its writers. When a mutating ask lands - create, forget, revoke a drive - it
//! needs its own admission and its own audit, and this note is where that starts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::net::{UnixListener, UnixStream};

use arlen_permissions::peer_pidfd::PeerPidfd;

use crate::protocol::{handle_request, Request};

/// The largest accepted frame body. A bottle list is a handful of short strings
/// per bottle, so 64 KiB is generous; a larger declared length is refused before
/// anything is allocated for it.
pub const MAX_FRAME: usize = 64 * 1024;

/// The daemon's socket: the `ARLEN_BOTTLED_SOCKET` override, else
/// `$XDG_RUNTIME_DIR/arlen/bottled.sock`, else `/run/arlen/bottled.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ARLEN_BOTTLED_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("bottled.sock")
}

/// Read one length-prefixed JSON frame, refusing an over-long one before
/// allocating for it.
pub async fn read_frame<R, T>(reader: &mut R) -> std::io::Result<T>
where
    R: tokio::io::AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write one length-prefixed JSON frame.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Serialize,
{
    use tokio::io::AsyncWriteExt;
    let body = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// Field requests on one connection until the peer closes or dies.
pub async fn serve_connection(mut stream: UnixStream, bottles_dir: &Path, caller_uid: u32) {
    let peer = match PeerPidfd::from_socket(&stream, caller_uid) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("peer auth refused: {e}");
            return;
        }
    };
    loop {
        // Re-checked per request rather than once: a pid that has been recycled
        // must not inherit the session the original process opened.
        if !peer.is_alive() {
            tracing::warn!("peer no longer alive; dropping");
            return;
        }
        let request: Request = match read_frame(&mut stream).await {
            Ok(r) => r,
            // A closed connection or a frame this cannot read ends the session.
            Err(_) => return,
        };
        let response = handle_request(bottles_dir, &request);
        if write_frame(&mut stream, &response).await.is_err() {
            return;
        }
    }
}

/// Bind the socket 0600 and serve until the future is dropped.
pub async fn run(socket: &Path, bottles_dir: PathBuf) -> std::io::Result<()> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A stale socket from a killed run would otherwise make the bind fail and the
    // daemon look broken on every restart.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    }
    let uid = current_uid();
    tracing::info!(socket = %socket.display(), "bottled listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let dir = bottles_dir.clone();
        tokio::spawn(async move {
            serve_connection(stream, &dir, uid).await;
        });
    }
}

/// This process's uid, which every admitted peer must share.
fn current_uid() -> u32 {
    // SAFETY: `getuid` reads a process property and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_frame_survives_the_round_trip_and_an_over_long_one_is_refused() {
        let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
        write_frame(&mut a, &Request::ListBottles).await.unwrap();
        let back: Request = read_frame(&mut b).await.unwrap();
        assert_eq!(back, Request::ListBottles);

        // A declared length beyond the cap is refused before the body is read, so
        // a caller cannot make this process allocate on its say-so.
        use tokio::io::AsyncWriteExt;
        a.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        assert!(read_frame::<_, Request>(&mut b).await.is_err());
    }
}
