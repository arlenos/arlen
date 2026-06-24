//! The real [`ConsentDriver`]: the requester side of the #9 consent-broker's
//! intake protocol (`pi-agent-adoption.md` §A Confirm verb / system-dialog-plan.md).
//!
//! The dispatcher resolves a gate `Confirm` by calling [`ConsentDriver::confirm`];
//! this implementation frames the confirmation as a [`RequestBody`] over the
//! broker's intake socket and reads back the single [`IntakeResult`] frame the
//! broker delivers once the user resolves the trusted-path dialog (the broker
//! parks the request and blocks this connection until then). The answer maps to
//! `Approved` / `Denied`.
//!
//! Wire format mirrors the broker's `socket.rs`: a 4-byte little-endian length
//! prefix then the JSON body, both directions, capped at [`MAX_FRAME`]. The
//! requester carries NO identity field (the broker attests the peer via
//! SO_PEERCRED), so this client cannot ask on another app's behalf.
//!
//! Fail-closed: a broker that is down, an I/O or framing error, or an oversized
//! reply resolves to [`ConfirmAnswer::Denied`] - a confirmation that cannot be
//! obtained is a denial, never a silent approval. The wait for the user's answer
//! is intentionally unbounded (a consent dialog blocks until the user decides);
//! only reaching the broker is what can fail fast.

use crate::capability_map::{action_kind_for_tool, consent_class_for_tool};
use crate::consent::ConsentDriver;
use ai_engine_contract::ConfirmAnswer;
use arlen_consent_contract::{ConsentOutcome, IntakeResult, RequestBody};
use async_trait::async_trait;
use std::io;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// The largest intake reply frame accepted, matching the broker's `MAX_FRAME`.
const MAX_FRAME: usize = 64 * 1024;

/// A consent-broker client that drives the trusted-path dialog over the intake
/// socket and returns the user's decision.
pub struct ConsentBrokerClient {
    socket_path: PathBuf,
}

impl ConsentBrokerClient {
    /// Build a client targeting the broker's intake socket.
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Frame `body` to the broker and read back its single [`IntakeResult`].
    async fn request(&self, body: &RequestBody) -> io::Result<IntakeResult> {
        let bytes = serde_json::to_vec(body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = u32::try_from(bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request too large"))?;

        let mut stream = UnixStream::connect(&self.socket_path).await?;
        stream.write_all(&len.to_le_bytes()).await?;
        stream.write_all(&bytes).await?;
        stream.flush().await?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let rlen = u32::from_le_bytes(len_buf) as usize;
        if rlen == 0 || rlen > MAX_FRAME {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "reply length out of bounds"));
        }
        let mut rbody = vec![0u8; rlen];
        stream.read_exact(&mut rbody).await?;
        serde_json::from_slice(&rbody)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

#[async_trait]
impl ConsentDriver for ConsentBrokerClient {
    async fn confirm(
        &self,
        tool_name: &str,
        prompt: &str,
        external_triggered: bool,
    ) -> ConfirmAnswer {
        let body = RequestBody {
            // Presentation: the dialog class matched to the tool (Destructive /
            // ExternalSend / ...) for the right copy, falling back to AgentAction.
            class: consent_class_for_tool(tool_name),
            // The same classifier the gate decided on, so the consent dialog's
            // severity matches the gate's verdict (no drift). The gate only
            // reaches Confirm for a high-impact or externally-triggered action,
            // so this never under-classifies into a silent grant.
            kind: action_kind_for_tool(tool_name),
            triggered_by_external_content: external_triggered,
            summary: prompt.to_string(),
            scope: Some(tool_name.to_string()),
        };
        match self.request(&body).await {
            Ok(IntakeResult::SilentGranted) => ConfirmAnswer::Approved,
            Ok(IntakeResult::Decided { outcome }) => match outcome {
                ConsentOutcome::AllowedOnce | ConsentOutcome::AllowedRemembered => {
                    ConfirmAnswer::Approved
                }
                ConsentOutcome::Denied => ConfirmAnswer::Denied,
            },
            // Broker unreachable / framing / I/O error: fail closed.
            Err(_) => ConfirmAnswer::Denied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_consent_contract::ConsentClass;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    /// A one-shot mock broker: accept one connection, read the framed
    /// `RequestBody`, hand it to `inspect`, and reply with `reply` (or, when
    /// `reply` is `None`, drop the connection to simulate a broker that dies).
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
            // dropping `stream` closes it (the broker-died case)
        })
    }

    fn sock(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("arlen-consent-client-test-{}-{}.sock", std::process::id(), name));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn a_decided_allow_becomes_approved_and_carries_the_request() {
        let path = sock("allow");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen2 = seen.clone();
        let broker = mock_broker(
            path.clone(),
            Some(IntakeResult::Decided { outcome: ConsentOutcome::AllowedOnce }),
            move |body| *seen2.lock().unwrap() = Some((body.class, body.summary, body.scope, body.triggered_by_external_content)),
        )
        .await;

        let client = ConsentBrokerClient::new(path.clone());
        let answer = client.confirm("send_email", "Confirm send_email?", true).await;
        assert_eq!(answer, ConfirmAnswer::Approved);
        broker.await.unwrap();

        // The broker received a request whose class is matched to the tool
        // (send_email -> ExternalSend), carrying the prompt as the summary, the
        // tool as the scope, and the external flag.
        let (class, summary, scope, external) = seen.lock().unwrap().clone().unwrap();
        assert_eq!(class, ConsentClass::ExternalSend);
        assert_eq!(summary, "Confirm send_email?");
        assert_eq!(scope.as_deref(), Some("send_email"));
        assert!(external);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_remembered_allow_is_approved() {
        let path = sock("remembered");
        let broker = mock_broker(
            path.clone(),
            Some(IntakeResult::Decided { outcome: ConsentOutcome::AllowedRemembered }),
            |_| {},
        )
        .await;
        let answer = ConsentBrokerClient::new(path.clone()).confirm("t", "p", false).await;
        assert_eq!(answer, ConfirmAnswer::Approved);
        broker.await.unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_silent_grant_is_approved() {
        let path = sock("silent");
        let broker = mock_broker(path.clone(), Some(IntakeResult::SilentGranted), |_| {}).await;
        let answer = ConsentBrokerClient::new(path.clone()).confirm("t", "p", false).await;
        assert_eq!(answer, ConfirmAnswer::Approved);
        broker.await.unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_denied_decision_is_denied() {
        let path = sock("denied");
        let broker = mock_broker(
            path.clone(),
            Some(IntakeResult::Decided { outcome: ConsentOutcome::Denied }),
            |_| {},
        )
        .await;
        let answer = ConsentBrokerClient::new(path.clone()).confirm("t", "p", false).await;
        assert_eq!(answer, ConfirmAnswer::Denied);
        broker.await.unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn an_unreachable_broker_fails_closed() {
        // Nothing bound at this path: connect fails -> Denied.
        let path = sock("absent");
        let answer = ConsentBrokerClient::new(path).confirm("t", "p", false).await;
        assert_eq!(answer, ConfirmAnswer::Denied);
    }

    #[tokio::test]
    async fn a_broker_that_dies_before_replying_fails_closed() {
        let path = sock("died");
        let broker = mock_broker(path.clone(), None, |_| {}).await;
        let answer = ConsentBrokerClient::new(path.clone()).confirm("t", "p", false).await;
        assert_eq!(answer, ConfirmAnswer::Denied);
        broker.await.unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
