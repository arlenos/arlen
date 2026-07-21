//! The contract socket's wire framing (`pi-agent-adoption.md` §A channels).
//!
//! The engine's plugins call the daemon over a Unix socket with length-prefixed
//! JSON frames: a 4-byte little-endian length, then that many UTF-8 bytes of a
//! [`ContractCall`]; the daemon replies with the same framing carrying a
//! [`Reply`]. The same shape `modulesd-proto`/`audit-proto` use. The frame
//! length is bounded so a malformed prefix cannot make the daemon allocate an
//! unbounded buffer.
//!
//! `serve_connection` is generic over the stream and takes the SO_PEERCRED pid
//! the accept loop already resolved, so it is unit-tested over a `UnixStream`
//! pair without a real socket. The accept loop + `ConnectionAuth` binding (which
//! resolves that pid) is the daemon binary's glue.

use crate::dispatch::{Dispatcher, Executor, Gate, Reporter};
use ai_engine_contract::{ContractCall, ContractError, Reply};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The largest contract frame the daemon will read (a generous bound; a single
/// call is small, this only stops a hostile length prefix from over-allocating).
pub const MAX_FRAME: usize = 256 * 1024;

/// Read one length-prefixed frame. `Ok(None)` is a clean EOF at a frame
/// boundary; an over-`MAX_FRAME` length or invalid UTF-8 is an error.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Option<String>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("contract frame too large: {len} > {MAX_FRAME}"),
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write one length-prefixed frame. Refuses a body over `MAX_FRAME`.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, body: &str) -> std::io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "contract reply too large",
        ));
    }
    writer.write_all(&(body.len() as u32).to_le_bytes()).await?;
    writer.write_all(body.as_bytes()).await?;
    writer.flush().await
}

/// Serve one authenticated engine connection: read [`ContractCall`] frames,
/// route each through the dispatcher with the connection's SO_PEERCRED `pid`,
/// and write the [`Reply`]. A malformed frame replies with a contract-level
/// `Error` (it does not desync the stream by closing mid-protocol). Returns when
/// the peer closes the connection at a frame boundary.
pub async fn serve_connection<S, G, E, R>(
    stream: &mut S,
    dispatcher: &Dispatcher<G, E, R>,
    pid: u32,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    G: Gate,
    E: Executor,
    R: Reporter,
{
    while let Some(frame) = read_frame(stream).await? {
        let reply = match serde_json::from_str::<ContractCall>(&frame) {
            Ok(call) => dispatcher.handle_call(call, pid).await,
            // A call we cannot parse is a contract-level error, not a session
            // failure; reply and keep the connection for the next frame.
            Err(_) => Reply::Error { code: ContractError::InvalidArguments },
        };
        let body = serde_json::to_string(&reply)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_frame(stream, &body).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::{Executor, Gate, Reporter};
    use crate::session::SessionGrant;
    use ai_engine_contract::{
        Authorize, AuthorizeDecision, Call, CapabilityContext, ContractCall, Execute, ExecuteOutcome,
        ReadTier, Report, ReportAck, ScreenVerdict, SessionInit,
    };
    use async_trait::async_trait;
    use tokio::net::UnixStream;

    struct AllowGate;
    #[async_trait]
    impl Gate for AllowGate {
        async fn authorize(&self, _: &Authorize, _: &SessionGrant) -> AuthorizeDecision {
            AuthorizeDecision::Allow { proof: None }
        }
    }
    struct OkExec;
    #[async_trait]
    impl Executor for OkExec {
        async fn execute(&self, _: &Execute, _: &SessionGrant) -> ExecuteOutcome {
            ExecuteOutcome::Ok { result: serde_json::json!("done") }
        }
    }
    struct CleanReporter;
    #[async_trait]
    impl Reporter for CleanReporter {
        async fn report(&self, _: &Report, _: &SessionGrant) -> ReportAck {
            ReportAck { screen: ScreenVerdict::Clean }
        }
    }

    fn dispatcher() -> Dispatcher<AllowGate, OkExec, CleanReporter> {
        Dispatcher::new(AllowGate, OkExec, CleanReporter)
    }

    fn init() -> SessionInit {
        SessionInit {
            system_prompt: "p".into(),
            behaviour: None,
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::Minimal,
            externally_triggered: false,
        }
    }

    #[test]
    fn frames_round_trip_through_a_buffer() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let mut buf: Vec<u8> = Vec::new();
            write_frame(&mut buf, "hello").await.unwrap();
            write_frame(&mut buf, "world").await.unwrap();
            let mut slice = &buf[..];
            assert_eq!(read_frame(&mut slice).await.unwrap().as_deref(), Some("hello"));
            assert_eq!(read_frame(&mut slice).await.unwrap().as_deref(), Some("world"));
            assert_eq!(read_frame(&mut slice).await.unwrap(), None); // clean EOF
        });
    }

    #[test]
    fn an_oversized_length_prefix_is_refused() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let mut bytes: Vec<u8> = ((MAX_FRAME as u32) + 1).to_le_bytes().to_vec();
            bytes.extend_from_slice(b"x");
            let mut slice = &bytes[..];
            assert!(read_frame(&mut slice).await.is_err());
        });
    }

    #[tokio::test]
    async fn an_authorized_call_is_served_over_a_socket_pair() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let disp = dispatcher();
        // The daemon would mint this at session start, bound to the engine pid.
        let token = disp.init_session(&init(), std::process::id()).unwrap();

        let server_task = tokio::spawn(async move {
            serve_connection(&mut server, &disp, std::process::id()).await.unwrap();
        });

        // The engine sends an Authorize call carrying its token.
        let call = ContractCall {
            token: token.as_str().to_string(),
            call: Call::Authorize(Authorize {
                tool_name: "bash".into(),
                tool_input: serde_json::json!({}),
                external_triggered: false,
            }),
        };
        let body = serde_json::to_string(&call).unwrap();
        write_frame(&mut client, &body).await.unwrap();

        let reply_frame = read_frame(&mut client).await.unwrap().unwrap();
        let reply: Reply = serde_json::from_str(&reply_frame).unwrap();
        assert!(matches!(reply, Reply::Authorize(AuthorizeDecision::Allow { proof: Some(_) })), "the wire reply carries a minted proof");

        drop(client); // EOF -> the server loop returns
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn a_malformed_frame_gets_a_contract_error_not_a_disconnect() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let disp = dispatcher();
        let server_task = tokio::spawn(async move {
            serve_connection(&mut server, &disp, std::process::id()).await.unwrap();
        });

        write_frame(&mut client, "{not valid json").await.unwrap();
        let reply: Reply = serde_json::from_str(&read_frame(&mut client).await.unwrap().unwrap()).unwrap();
        assert_eq!(reply, Reply::Error { code: ContractError::InvalidArguments });

        // The connection survives: a following valid (but session-less) call still answers.
        let call = ContractCall {
            token: "nope".into(),
            call: Call::Authorize(Authorize { tool_name: "x".into(), tool_input: serde_json::json!({}), external_triggered: false }),
        };
        write_frame(&mut client, &serde_json::to_string(&call).unwrap()).await.unwrap();
        let reply: Reply = serde_json::from_str(&read_frame(&mut client).await.unwrap().unwrap()).unwrap();
        assert!(matches!(reply, Reply::Authorize(AuthorizeDecision::Deny { .. })));

        drop(client);
        server_task.await.unwrap();
    }
}
