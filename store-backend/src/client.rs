//! The `org.arlen.Store1` client (store-app.md section 9.4): the caller-side
//! mirror of [`crate::serve`]. Speaks the same frame (a 4-byte big-endian length
//! header then a JSON body), one request/response roundtrip per call, bounding
//! both frames by the same limits the server enforces so a buggy or hostile peer
//! cannot make the client allocate without bound. The store app's `src-tauri`
//! proxy commands are the primary caller; the wire format lives in exactly one
//! crate so it cannot drift from the server.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::query::{Request, Response};
use crate::serve::{socket_path, MAX_REQUEST_FRAME, MAX_RESPONSE_FRAME};

/// A client-side failure talking to the store backend.
#[derive(Debug)]
pub enum ClientError {
    /// The backend socket path could not be resolved (`$XDG_RUNTIME_DIR` unset).
    NoSocket,
    /// A transport error (connect, read or write).
    Io(String),
    /// A framing violation: a length header outside the agreed bounds.
    Frame(String),
    /// The response body was not valid `Response` JSON, or the request could not
    /// be serialized.
    Codec(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NoSocket => write!(f, "store socket path unresolved"),
            ClientError::Io(e) => write!(f, "store transport error: {e}"),
            ClientError::Frame(e) => write!(f, "store frame error: {e}"),
            ClientError::Codec(e) => write!(f, "store codec error: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Send one request to the store backend at the default socket
/// (`$XDG_RUNTIME_DIR/arlen/store.sock`) and return its response. A missing
/// runtime dir yields [`ClientError::NoSocket`]; a down backend yields
/// [`ClientError::Io`].
pub async fn request_default(req: &Request) -> Result<Response, ClientError> {
    let socket = socket_path().ok_or(ClientError::NoSocket)?;
    request(&socket, req).await
}

/// Send one request over a fresh connection to `socket` and return the response.
pub async fn request(socket: &Path, req: &Request) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| ClientError::Io(e.to_string()))?;
    roundtrip(&mut stream, req).await
}

/// Write one request and read one response on an already-connected stream. Split
/// out from [`request`] so the framing is unit-tested over an in-memory pair,
/// without binding a socket file.
async fn roundtrip<S>(stream: &mut S, req: &Request) -> Result<Response, ClientError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(req).map_err(|e| ClientError::Codec(e.to_string()))?;
    if body.is_empty() || body.len() > MAX_REQUEST_FRAME {
        return Err(ClientError::Frame(format!(
            "request length {} out of range (1..={MAX_REQUEST_FRAME})",
            body.len()
        )));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| ClientError::Frame("request exceeds u32 length".to_string()))?;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| ClientError::Io(e.to_string()))?;
    stream.write_all(&body).await.map_err(|e| ClientError::Io(e.to_string()))?;
    stream.flush().await.map_err(|e| ClientError::Io(e.to_string()))?;

    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|e| ClientError::Io(e.to_string()))?;
    let resp_len = u32::from_be_bytes(header) as usize;
    if resp_len == 0 || resp_len > MAX_RESPONSE_FRAME {
        return Err(ClientError::Frame(format!(
            "response length {resp_len} out of range (1..={MAX_RESPONSE_FRAME})"
        )));
    }
    let mut resp_body = vec![0u8; resp_len];
    stream
        .read_exact(&mut resp_body)
        .await
        .map_err(|e| ClientError::Io(e.to_string()))?;
    serde_json::from_slice(&resp_body).map_err(|e| ClientError::Codec(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        merge_catalog, CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, ItemKind, SourceLayer,
        TrustSignals,
    };
    use crate::query::Catalog;
    use crate::serve::serve_connection;

    fn fixture_catalog() -> Catalog {
        let entry = CatalogEntry {
            id: ComponentId("org.demo.App".into()),
            layer: SourceLayer::Official,
            display: DisplayMeta { name: "Demo".into(), ..Default::default() },
            capabilities: CapabilityFootprint::default(),
            trust: TrustSignals::default(),
            kind: ItemKind::default(),
            version: String::new(),
            install_handle: None,
        };
        Catalog::new(merge_catalog(vec![entry]))
    }

    /// A real request/response over a duplex pair against the actual server loop:
    /// a wrong length width or endianness would break this, so it pins the frame.
    #[tokio::test]
    async fn a_search_roundtrips_against_the_server() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let handle = tokio::spawn(async move {
            let cat = fixture_catalog();
            let _ = serve_connection(&mut server, &cat).await;
        });

        let resp = roundtrip(
            &mut client,
            &Request::Search { query: "demo".into(), facets: vec![], sort: Default::default() },
        )
        .await
        .unwrap();
        match resp {
            Response::Cards(cards) => {
                assert_eq!(cards.len(), 1);
                assert_eq!(cards[0].display.name, "Demo");
            }
            other => panic!("expected Cards, got {other:?}"),
        }
        drop(client);
        handle.await.unwrap();
    }

    /// An unknown id resolves to `Card(None)` (not an error), so the app renders a
    /// clean "not found" rather than a failure.
    #[tokio::test]
    async fn an_unknown_app_detail_roundtrips_to_none() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let handle = tokio::spawn(async move {
            let cat = fixture_catalog();
            let _ = serve_connection(&mut server, &cat).await;
        });

        let resp = roundtrip(
            &mut client,
            &Request::AppDetail { id: ComponentId("org.nope.Missing".into()) },
        )
        .await
        .unwrap();
        assert_eq!(resp, Response::Card(None));
        drop(client);
        handle.await.unwrap();
    }
}
