/// End-to-end smoke test for the Unix socket framing + manager
/// dispatch. Spawns the daemon's socket server in-process, connects a
/// raw client, sends a `Hello`, expects a `Hello` back. This covers
/// the protocol layer without any WASM dependency.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

use lunaris_modulesd::manager::Manager;
use lunaris_modulesd::socket::protocol::{Request, Response};
use lunaris_modulesd::socket::server::SocketServer;

async fn write_frame(stream: &mut UnixStream, body: &[u8]) -> std::io::Result<()> {
    let len = (body.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

async fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    let mut body = vec![0u8; n];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

#[tokio::test]
async fn hello_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("modulesd.sock");

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx.clone()).unwrap();
    let server = SocketServer::bind(&socket_path, Arc::clone(&manager), tx).unwrap();

    let server_handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Give the listener a moment to come up. Polling here would be
    // tidier; for the smoke test a short yield suffices.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&socket_path).await.unwrap();
    let req = Request::Hello {
        id: "1".into(),
        client: "test".into(),
        version: "0.0.1".into(),
    };
    let body = serde_json::to_vec(&req).unwrap();
    write_frame(&mut client, &body).await.unwrap();

    let reply = read_frame(&mut client).await.unwrap();
    let parsed: Response = serde_json::from_slice(&reply).unwrap();
    assert!(matches!(parsed, Response::Hello { .. }));

    server_handle.abort();
}

#[tokio::test]
async fn list_modules_returns_empty_when_no_modules_discovered() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("modulesd.sock");

    let (tx, _rx) = broadcast::channel(16);
    let manager: Arc<Manager> = Manager::new(tx.clone()).unwrap();
    let server = SocketServer::bind(&socket_path, Arc::clone(&manager), tx).unwrap();

    let server_handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&socket_path).await.unwrap();
    let req = Request::ListModules { id: "1".into() };
    let body = serde_json::to_vec(&req).unwrap();
    write_frame(&mut client, &body).await.unwrap();

    let reply = read_frame(&mut client).await.unwrap();
    let parsed: Response = serde_json::from_slice(&reply).unwrap();
    match parsed {
        Response::ModuleList { modules, .. } => assert!(modules.is_empty()),
        other => panic!("unexpected reply: {other:?}"),
    }

    server_handle.abort();
}
