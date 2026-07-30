//! Asking the consent broker, from the command line.
//!
//! `forage` performs installs, and some of those write authority - a bridge's
//! delegated namespace grant most of all. The broker owns the trusted-path
//! dialog for that decision, so this is the client, not a prompt of its own: a
//! CLI reading a yes/no on its own stdin would be a second consent surface, and
//! the whole point of the broker is that there is one.
//!
//! Fail-closed. `None` means no decision could be obtained, and every caller
//! reads that as a refusal.

use std::path::PathBuf;

use arlen_consent_contract::{IntakeResult, RequestBody};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// The largest reply frame accepted, matching the broker's own bound.
const MAX_FRAME: usize = 64 * 1024;

/// The broker's intake socket, mirroring its bind.
fn intake_socket() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("consent-intake.sock")
}

/// Put `body` to the broker and wait for the user's decision.
///
/// Blocks for as long as the dialog is open - a consent prompt has no timeout
/// by design, since the alternative is deciding on the user's behalf because
/// they were slow. Only reaching the broker can fail fast.
pub async fn ask(body: &RequestBody) -> Option<IntakeResult> {
    match exchange(&intake_socket(), body).await {
        Ok(result) => Some(result),
        Err(e) => {
            eprintln!("could not obtain consent: {e}");
            None
        }
    }
}

/// One intake round trip: a 4-byte little-endian length then JSON, both ways.
async fn exchange(socket: &std::path::Path, body: &RequestBody) -> Result<IntakeResult, String> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("the consent broker is not reachable: {e}"))?;

    let payload = serde_json::to_vec(body).map_err(|e| format!("encoding the request: {e}"))?;
    let len = u32::try_from(payload.len()).map_err(|_| "request too large".to_string())?;
    stream
        .write_all(&len.to_le_bytes())
        .await
        .map_err(|e| format!("writing the request: {e}"))?;
    stream
        .write_all(&payload)
        .await
        .map_err(|e| format!("writing the request: {e}"))?;

    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("reading the reply: {e}"))?;
    // Checked before allocating, so a corrupt length cannot make us reserve it.
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(format!("reply frame {len} exceeds {MAX_FRAME}"));
    }
    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("reading the reply: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("decoding the reply: {e}"))
}
