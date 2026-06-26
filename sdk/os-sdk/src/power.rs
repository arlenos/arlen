//! Power-state query client (SST-R3, the socket half of the read API).
//!
//! The power-daemon publishes `power.state` transitions to the event bus (push)
//! and serves the latest snapshot over `org.arlen.Power1` D-Bus properties. This
//! is the third surface: a one-shot **query socket** so an os-sdk app pulls the
//! current state on demand in the SDK's native socket style, without speaking
//! D-Bus or subscribing to the bus. The daemon writes one [`PowerSnapshot`] as
//! JSON on accept and closes; the field names and value strings match the D-Bus
//! properties and the `power.state` event payload, so a caller gets the same
//! shape by push or pull.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

/// The coarse power snapshot the daemon serves on its query socket.
///
/// `charge_state` is one of `charging`/`discharging`/`full`/`empty`/`unknown`;
/// `lid_state` is `open`/`closed`/`none`; `profile` is `performance`/`balanced`/
/// `power-saver`/`unknown`. The time fields are `0` when not meaningful (charging
/// has no time-to-empty and vice versa).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerSnapshot {
    /// True on battery, false on AC.
    pub on_battery: bool,
    /// Battery charge, 0-100.
    pub percentage: u8,
    /// Charge state string.
    pub charge_state: String,
    /// Seconds to empty (0 when unknown or charging).
    pub time_to_empty_seconds: i64,
    /// Seconds to full (0 when unknown or discharging).
    pub time_to_full_seconds: i64,
    /// Lid state string.
    pub lid_state: String,
    /// Active power profile string.
    pub profile: String,
}

/// The query socket path (`ARLEN_POWER_SOCKET`, else `power.sock` under the
/// per-user runtime dir).
pub fn socket_path() -> PathBuf {
    crate::runtime::socket_path("ARLEN_POWER_SOCKET", "power.sock")
}

/// Why a power-state query failed.
#[derive(Debug, thiserror::Error)]
pub enum PowerError {
    /// The query socket could not be reached (daemon down / wrong path).
    #[error("connect to power daemon: {0}")]
    Connect(std::io::Error),
    /// The connection dropped before a full snapshot was read.
    #[error("read power snapshot: {0}")]
    Read(std::io::Error),
    /// The bytes were not a valid [`PowerSnapshot`] JSON document.
    #[error("decode power snapshot: {0}")]
    Decode(serde_json::Error),
    /// The daemon sent more than the snapshot bound; a well-behaved daemon never
    /// does, so an over-long stream is rejected rather than buffered unbounded.
    #[error("power snapshot exceeded {0} bytes")]
    TooLarge(usize),
}

/// The most a single snapshot can occupy on the wire. A `PowerSnapshot` is a
/// handful of short fields; this cap bounds the read so a misbehaving or hostile
/// peer cannot make the client buffer without limit.
const MAX_SNAPSHOT_BYTES: usize = 4096;

/// Query the current power snapshot from the default socket. Connects, reads the
/// JSON the daemon writes on accept, and decodes it. One-shot: the connection is
/// closed after the read.
pub async fn read_power_state() -> Result<PowerSnapshot, PowerError> {
    read_power_state_at(&socket_path()).await
}

/// Like [`read_power_state`] against an explicit socket path (tests, or a caller
/// that pins the path).
pub async fn read_power_state_at(path: &Path) -> Result<PowerSnapshot, PowerError> {
    let mut stream = UnixStream::connect(path).await.map_err(PowerError::Connect)?;
    // The daemon writes one snapshot then closes, so read to EOF under a cap.
    let mut buf = Vec::with_capacity(256);
    let mut chunk = [0u8; 512];
    loop {
        let n = stream.read(&mut chunk).await.map_err(PowerError::Read)?;
        if n == 0 {
            break;
        }
        if buf.len() + n > MAX_SNAPSHOT_BYTES {
            return Err(PowerError::TooLarge(MAX_SNAPSHOT_BYTES));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    serde_json::from_slice(&buf).map_err(PowerError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;

    fn sample() -> PowerSnapshot {
        PowerSnapshot {
            on_battery: true,
            percentage: 73,
            charge_state: "discharging".to_string(),
            time_to_empty_seconds: 5400,
            time_to_full_seconds: 0,
            lid_state: "open".to_string(),
            profile: "balanced".to_string(),
        }
    }

    #[tokio::test]
    async fn reads_a_snapshot_a_fake_daemon_serves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("power.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let want = sample();
        let served = want.clone();
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let json = serde_json::to_vec(&served).unwrap();
            conn.write_all(&json).await.unwrap();
            // Drop closes the connection, signalling EOF to the reader.
        });
        let got = read_power_state_at(&path).await.unwrap();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn a_missing_socket_is_a_connect_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.sock");
        let err = read_power_state_at(&path).await.unwrap_err();
        assert!(matches!(err, PowerError::Connect(_)));
    }

    #[tokio::test]
    async fn an_oversized_stream_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flood.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let flood = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
            let _ = conn.write_all(&flood).await;
        });
        let err = read_power_state_at(&path).await.unwrap_err();
        assert!(matches!(err, PowerError::TooLarge(_)));
    }
}
