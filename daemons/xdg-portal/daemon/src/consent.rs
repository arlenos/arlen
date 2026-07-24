//! Capture-active #12: the ScreenCast backend's consent-broker client.
//!
//! Before a screencast streams, the portal asks the consent broker to authorise
//! it. Because the capturing app reaches the portal FRONTEND (not the broker
//! directly), the portal is a trusted intermediary: it raises the request ON
//! BEHALF OF the app (the frontend-verified `app_id`), so the grant + the dialog
//! name the APP, not the portal (the broker honors `on_behalf_of` only for the
//! allowlisted, SO_PEERCRED-attested portal). The broker parks the trusted-path
//! dialog and blocks this connection until the user decides.
//!
//! FAIL-CLOSED: a broker that is down, a framing/IO error, or an oversized reply
//! resolves to [`ConsentDecision::Denied`] - a capture that cannot obtain consent
//! never proceeds. The wire format mirrors the broker's intake socket (a 4-byte
//! little-endian length prefix then JSON, both directions).

use std::path::PathBuf;

use arlen_consent_contract::{
    ActionKind, ConsentClass, ConsentOutcome, IntakeResult, RequestBody,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// The largest intake reply frame accepted, matching the broker's `MAX_FRAME`.
const MAX_FRAME: usize = 64 * 1024;

/// Whether a capture may proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    /// The user (or a remembered grant) allowed it.
    Allowed,
    /// Denied, or consent could not be obtained (fail-closed).
    Denied,
}

/// The broker's intake socket: `$XDG_RUNTIME_DIR/arlen/consent-intake.sock`,
/// else `/run/arlen/consent-intake.sock`. Mirrors the broker's bind.
pub fn intake_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("consent-intake.sock")
}

/// Ask the broker to authorise a screencast on behalf of `app_id`. Blocks until
/// the user resolves the dialog (a consent prompt has no timeout by design);
/// only reaching the broker can fail fast, and any failure is a denial.
pub async fn request_screencast_consent(socket: &PathBuf, app_id: &str, summary: &str) -> ConsentDecision {
    let body = RequestBody {
        class: ConsentClass::ScreenCast,
        // A screencast is reversible (the CaptureBadge + revoke are the safety
        // net), so it classifies to a standard prompt, never a silent grant.
        kind: ActionKind::Ordinary,
        triggered_by_external_content: false,
        summary: summary.to_string(),
        scope: None,
        recipient: None,
        preview: None,
        targets: Vec::new(),
        total: None,
        // The trusted-intermediary attribution: the grant is for the capturing
        // app, not the portal. The broker ignores this unless the attested peer
        // is the allowlisted portal.
        on_behalf_of: Some(app_id.to_string()),
    };
    match request(socket, &body).await {
        Ok(IntakeResult::SilentGranted) => ConsentDecision::Allowed,
        Ok(IntakeResult::Decided { outcome }) => match outcome {
            ConsentOutcome::AllowedOnce | ConsentOutcome::AllowedRemembered => {
                ConsentDecision::Allowed
            }
            ConsentOutcome::Denied => ConsentDecision::Denied,
        },
        // Broker unreachable / framing / IO error: fail closed.
        Err(e) => {
            tracing::warn!("screencast consent request failed, denying: {e}");
            ConsentDecision::Denied
        }
    }
}

/// Frame `body` to the broker's intake socket and read back its single reply.
async fn request(socket: &PathBuf, body: &RequestBody) -> std::io::Result<IntakeResult> {
    use std::io::{Error, ErrorKind};
    let bytes = serde_json::to_vec(body).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let len =
        u32::try_from(bytes.len()).map_err(|_| Error::new(ErrorKind::InvalidData, "request too large"))?;

    let mut stream = UnixStream::connect(socket).await?;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let rlen = u32::from_le_bytes(len_buf) as usize;
    if rlen == 0 || rlen > MAX_FRAME {
        return Err(Error::new(ErrorKind::InvalidData, "reply length out of bounds"));
    }
    let mut rbody = vec![0u8; rlen];
    stream.read_exact(&mut rbody).await?;
    serde_json::from_slice(&rbody).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    /// A one-shot mock broker: accept one connection, read the framed
    /// `RequestBody`, hand it to `inspect`, and reply with `reply` (or drop the
    /// connection when `reply` is `None`, simulating a broker that dies).
    async fn mock_broker(
        path: PathBuf,
        reply: Option<IntakeResult>,
        inspect: impl FnOnce(RequestBody) + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut body = vec![0u8; len];
            stream.read_exact(&mut body).await.unwrap();
            inspect(serde_json::from_slice(&body).unwrap());
            if let Some(result) = reply {
                let bytes = serde_json::to_vec(&result).unwrap();
                stream.write_all(&(bytes.len() as u32).to_le_bytes()).await.unwrap();
                stream.write_all(&bytes).await.unwrap();
                stream.flush().await.unwrap();
            }
        })
    }

    fn sock(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("arlen-portal-consent-{}-{}.sock", std::process::id(), name));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn a_grant_carries_the_app_as_on_behalf_of_and_screencast_class() {
        let path = sock("allow");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen2 = seen.clone();
        let server = mock_broker(
            path.clone(),
            Some(IntakeResult::Decided { outcome: ConsentOutcome::AllowedRemembered }),
            move |body| *seen2.lock().unwrap() = Some(body),
        )
        .await;

        let decision = request_screencast_consent(&path, "org.example.recorder", "Share your screen").await;
        server.await.unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(decision, ConsentDecision::Allowed);
        let body = seen.lock().unwrap().take().unwrap();
        assert_eq!(body.class, ConsentClass::ScreenCast);
        assert_eq!(body.on_behalf_of.as_deref(), Some("org.example.recorder"));
    }

    #[tokio::test]
    async fn a_denial_is_denied() {
        let path = sock("deny");
        let server = mock_broker(
            path.clone(),
            Some(IntakeResult::Decided { outcome: ConsentOutcome::Denied }),
            |_| {},
        )
        .await;
        let decision = request_screencast_consent(&path, "org.example.recorder", "x").await;
        server.await.unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(decision, ConsentDecision::Denied);
    }

    #[tokio::test]
    async fn an_unreachable_broker_fails_closed() {
        // No socket bound at this path: the connect fails -> denied.
        let path = sock("absent");
        let _ = std::fs::remove_file(&path);
        let decision = request_screencast_consent(&path, "org.example.recorder", "x").await;
        assert_eq!(decision, ConsentDecision::Denied, "no consent obtainable must deny");
    }
}
