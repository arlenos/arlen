//! The `org.arlen.Store1` socket transport (store-app.md section 9.4): length-
//! prefixed request/response frames over an `AF_UNIX` session socket, the framing
//! convention the sibling daemons use (a 4-byte big-endian length header then a JSON
//! body). The request bound is small; the response bound is larger because a search
//! can return many cards.
//!
//! [`serve_connection`] is generic over any async stream, so a socket-pair round-trip
//! tests the whole read -> [`answer`] -> write path without binding a real socket. The
//! catalog is read-only and shared, so a connection never mutates store state.

use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::query::{answer, Catalog, Request, Response};

/// The largest request frame accepted (a request is small: a query + facets).
pub const MAX_REQUEST_FRAME: usize = 64 * 1024;
/// The largest response frame written (a search can return many cards).
pub const MAX_RESPONSE_FRAME: usize = 16 * 1024 * 1024;

/// Why a connection could not be served.
#[derive(Debug)]
pub enum ServeError {
    /// A transport read/write failure.
    Io(String),
    /// A frame whose declared length is zero or out of bounds.
    Frame(String),
    /// A request body that is not valid JSON, or a response that would not encode.
    Codec(String),
}

/// The session socket path (`$XDG_RUNTIME_DIR/arlen/store.sock`), or `None` if the
/// runtime dir is absent so the daemon fails closed rather than bind a stray path.
pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("arlen/store.sock"))
}

/// Read one length-prefixed request. `Ok(None)` on a clean close at a frame boundary
/// (the client hung up), so the serve loop ends without an error.
async fn read_request<S>(stream: &mut S) -> Result<Option<Request>, ServeError>
where
    S: AsyncReadExt + Unpin,
{
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(ServeError::Io(e.to_string())),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_REQUEST_FRAME {
        return Err(ServeError::Frame(format!(
            "request length {len} out of range (1..={MAX_REQUEST_FRAME})"
        )));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(|e| ServeError::Io(e.to_string()))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| ServeError::Codec(e.to_string()))
}

/// Write one length-prefixed response.
async fn write_response<S>(stream: &mut S, response: &Response) -> Result<(), ServeError>
where
    S: AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(response).map_err(|e| ServeError::Codec(e.to_string()))?;
    if body.is_empty() || body.len() > MAX_RESPONSE_FRAME {
        return Err(ServeError::Frame(format!(
            "response length {} out of range (1..={MAX_RESPONSE_FRAME})",
            body.len()
        )));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| ServeError::Frame("response exceeds u32 length".to_string()))?;
    stream.write_all(&len.to_be_bytes()).await.map_err(|e| ServeError::Io(e.to_string()))?;
    stream.write_all(&body).await.map_err(|e| ServeError::Io(e.to_string()))?;
    stream.flush().await.map_err(|e| ServeError::Io(e.to_string()))?;
    Ok(())
}

/// Serve one connection: read requests until the client hangs up, answering each
/// against the shared read-only `catalog`. A malformed frame ends the connection with
/// an error rather than a panic.
pub async fn serve_connection<S>(stream: &mut S, catalog: &Catalog) -> Result<(), ServeError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    while let Some(request) = read_request(stream).await? {
        let response = answer(catalog, request);
        write_response(stream, &response).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        merge_catalog, CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, SourceLayer,
        TrustSignals,
    };

    fn fixture_catalog() -> Catalog {
        let entry = CatalogEntry {
            id: ComponentId("org.demo.App".into()),
            layer: SourceLayer::Official,
            display: DisplayMeta { name: "Demo".into(), ..Default::default() },
            capabilities: CapabilityFootprint::default(),
            trust: TrustSignals::default(),
        };
        Catalog::new(merge_catalog(vec![entry]))
    }

    async fn write_request<S: AsyncWriteExt + Unpin>(stream: &mut S, req: &Request) {
        let body = serde_json::to_vec(req).unwrap();
        stream.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
        stream.write_all(&body).await.unwrap();
        stream.flush().await.unwrap();
    }

    async fn read_response<S: AsyncReadExt + Unpin>(stream: &mut S) -> Response {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.unwrap();
        let len = u32::from_be_bytes(header) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn a_request_is_answered_over_a_socket_pair() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let handle = tokio::spawn(async move {
            let cat = fixture_catalog();
            serve_connection(&mut server, &cat).await
        });

        write_request(&mut client, &Request::Search { query: "demo".into(), facets: vec![] }).await;
        match read_response(&mut client).await {
            Response::Cards(cards) => {
                assert_eq!(cards.len(), 1);
                assert_eq!(cards[0].display.name, "Demo");
            }
            other => panic!("expected Cards, got {other:?}"),
        }
        // A second request on the same connection is served too.
        write_request(&mut client, &Request::AppDetail { id: ComponentId("nope".into()) }).await;
        assert!(matches!(read_response(&mut client).await, Response::Card(None)));

        // Closing the client ends the serve loop cleanly.
        drop(client);
        assert!(handle.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn an_oversized_request_frame_is_refused() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let handle = tokio::spawn(async move {
            let cat = fixture_catalog();
            serve_connection(&mut server, &cat).await
        });
        // Declare a length past the request bound, then hang up.
        client
            .write_all(&((MAX_REQUEST_FRAME as u32 + 1).to_be_bytes()))
            .await
            .unwrap();
        client.flush().await.unwrap();
        drop(client);
        assert!(matches!(handle.await.unwrap(), Err(ServeError::Frame(_))));
    }
}
