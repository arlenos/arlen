/// Integration tests for the real UnixEventEmitter and UnixGraphClient implementations.
///
/// These tests spin up minimal Unix socket servers in-process rather than
/// starting real daemons. This keeps the tests fast and self-contained while
/// still testing the actual socket I/O, length-prefixed protocol, and
/// reconnect logic.

use os_sdk::event::{EmitError, EventEmitter, UnixEventEmitter};
use os_sdk::graph::{GraphClient, QueryError, UnixGraphClient};
use prost::Message as _;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

mod proto {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

/// A minimal Event Bus producer socket server that records received messages.
struct FakeEventBus {
    received: Arc<Mutex<Vec<proto::Event>>>,
    tmp: TempDir,
    socket_path: PathBuf,
}

impl FakeEventBus {
    fn start() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("producer.sock");
        let received: Arc<Mutex<Vec<proto::Event>>> = Arc::new(Mutex::new(Vec::new()));

        let listener = UnixListener::bind(&socket_path).expect("bind");
        let received_clone = received.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let received = received_clone.clone();
                std::thread::spawn(move || {
                    loop {
                        // Read 4-byte length prefix
                        let mut len_buf = [0u8; 4];
                        if stream.read_exact(&mut len_buf).is_err() {
                            break;
                        }
                        let len = u32::from_be_bytes(len_buf) as usize;
                        if len == 0 || len > 1024 * 1024 {
                            break;
                        }
                        let mut buf = vec![0u8; len];
                        if stream.read_exact(&mut buf).is_err() {
                            break;
                        }
                        if let Ok(event) = proto::Event::decode(buf.as_slice()) {
                            received.lock().unwrap().push(event);
                        }
                    }
                });
            }
        });

        // Give server time to start
        std::thread::sleep(Duration::from_millis(50));

        Self { received, tmp, socket_path }
    }

    fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap()
    }

    fn received_events(&self) -> Vec<proto::Event> {
        self.received.lock().unwrap().clone()
    }

    fn wait_for_events(&self, count: usize, timeout: Duration) -> Vec<proto::Event> {
        let start = std::time::Instant::now();
        loop {
            let events = self.received_events();
            if events.len() >= count || start.elapsed() > timeout {
                return events;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// A minimal Graph Daemon socket server that returns canned responses.
struct FakeGraphDaemon {
    tmp: TempDir,
    socket_path: PathBuf,
}

impl FakeGraphDaemon {
    fn start(response: &'static str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind");

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                std::thread::spawn(move || {
                    loop {
                        // Read query length
                        let mut len_buf = [0u8; 4];
                        if stream.read_exact(&mut len_buf).is_err() {
                            break;
                        }
                        let len = u32::from_be_bytes(len_buf) as usize;
                        if len == 0 || len > 64 * 1024 {
                            break;
                        }
                        let mut buf = vec![0u8; len];
                        if stream.read_exact(&mut buf).is_err() {
                            break;
                        }

                        // Send canned response
                        let resp = response.as_bytes();
                        let resp_len = (resp.len() as u32).to_be_bytes();
                        if stream.write_all(&resp_len).is_err() {
                            break;
                        }
                        if stream.write_all(resp).is_err() {
                            break;
                        }
                    }
                });
            }
        });

        std::thread::sleep(Duration::from_millis(50));
        Self { tmp, socket_path }
    }

    fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap()
    }
}

// ============================================================
// UnixEventEmitter tests
// ============================================================

#[tokio::test]
async fn emitter_sends_event_to_socket() {
    let bus = FakeEventBus::start();
    let emitter = UnixEventEmitter::new(bus.socket_path());

    emitter.emit("file.opened", vec![1, 2, 3]).await.unwrap();

    let events = bus.wait_for_events(1, Duration::from_secs(2));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].r#type, "file.opened");
    assert_eq!(events[0].payload, vec![1, 2, 3]);
}

#[tokio::test]
async fn emitter_sends_multiple_events() {
    let bus = FakeEventBus::start();
    let emitter = UnixEventEmitter::new(bus.socket_path());

    emitter.emit("window.focused", vec![]).await.unwrap();
    emitter.emit("file.opened", vec![]).await.unwrap();
    emitter.emit("clipboard.copy", vec![]).await.unwrap();

    let events = bus.wait_for_events(3, Duration::from_secs(2));
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].r#type, "window.focused");
    assert_eq!(events[1].r#type, "file.opened");
    assert_eq!(events[2].r#type, "clipboard.copy");
}

#[tokio::test]
async fn emitter_returns_error_when_socket_unavailable() {
    let emitter = UnixEventEmitter::new("/tmp/lunaris-nonexistent-socket.sock");
    let result = emitter.emit("file.opened", vec![]).await;
    assert!(matches!(result, Err(EmitError::ConnectionFailed(_))));
}

#[tokio::test]
async fn emitter_reconnects_after_server_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("producer.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    // Start first server
    let received: Arc<Mutex<Vec<proto::Event>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let listener = UnixListener::bind(&socket_path).unwrap();
        let received_clone = received.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut len_buf = [0u8; 4];
                if stream.read_exact(&mut len_buf).is_ok() {
                    let len = u32::from_be_bytes(len_buf) as usize;
                    let mut buf = vec![0u8; len];
                    if stream.read_exact(&mut buf).is_ok() {
                        if let Ok(event) = proto::Event::decode(buf.as_slice()) {
                            received_clone.lock().unwrap().push(event);
                        }
                    }
                }
                // Server closes connection here
            }
        });
    }

    std::thread::sleep(Duration::from_millis(50));

    let emitter = UnixEventEmitter::new(&path_str);
    emitter.emit("file.opened", vec![]).await.unwrap();

    // Wait for first server to process
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(received.lock().unwrap().len(), 1);

    // Remove old socket and start second server
    std::fs::remove_file(&socket_path).ok();
    let received2: Arc<Mutex<Vec<proto::Event>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let listener = UnixListener::bind(&socket_path).unwrap();
        let received_clone = received2.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut len_buf = [0u8; 4];
                if stream.read_exact(&mut len_buf).is_ok() {
                    let len = u32::from_be_bytes(len_buf) as usize;
                    let mut buf = vec![0u8; len];
                    if stream.read_exact(&mut buf).is_ok() {
                        if let Ok(event) = proto::Event::decode(buf.as_slice()) {
                            received_clone.lock().unwrap().push(event);
                        }
                    }
                }
            }
        });
    }

    std::thread::sleep(Duration::from_millis(50));

    // Emitter should reconnect automatically
    emitter.emit("window.focused", vec![]).await.unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(received2.lock().unwrap().len(), 1);
    assert_eq!(received2.lock().unwrap()[0].r#type, "window.focused");
}

// ============================================================
// UnixGraphClient tests
// ============================================================

#[tokio::test]
async fn graph_client_sends_query_and_receives_response() {
    let daemon = FakeGraphDaemon::start("result: 42 nodes");
    let client = UnixGraphClient::new(daemon.socket_path());

    let rows = client
        .query("MATCH (n) RETURN count(n)", HashMap::new())
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains_key("result"));
}

#[tokio::test]
async fn graph_client_returns_error_for_error_response() {
    let daemon = FakeGraphDaemon::start("ERROR: write queries are not permitted");
    let client = UnixGraphClient::new(daemon.socket_path());

    let result = client
        .query("CREATE (n:File)", HashMap::new())
        .await;

    assert!(matches!(result, Err(QueryError::InvalidQuery(_))));
}

#[tokio::test]
async fn graph_client_returns_error_when_socket_unavailable() {
    let client = UnixGraphClient::new("/tmp/lunaris-nonexistent-daemon.sock");
    let result = client
        .query("MATCH (n) RETURN n", HashMap::new())
        .await;
    assert!(matches!(result, Err(QueryError::ConnectionFailed(_))));
}
