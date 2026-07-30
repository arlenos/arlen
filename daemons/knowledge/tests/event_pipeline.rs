/// Integration test: event emitted by a producer lands in SQLite via the Event Bus.
///
/// This test starts real event-bus and knowledge daemon processes,
/// sends a synthetic event over the producer socket, waits for the
/// batch timer to fire, and verifies the event exists in SQLite.
///
/// Both binaries must be built before running this test:
///   cargo build --manifest-path ../event-bus/Cargo.toml
///   cargo build --manifest-path ../knowledge/Cargo.toml
///
/// The test uses temporary socket paths and a temporary database to
/// avoid interfering with a running system.
use prost::Message;
use sqlx::sqlite::SqlitePoolOptions;
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

// Include generated protobuf types.
// We build the proto in knowledge's build.rs so we can use them here too.
mod proto {
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

/// Locate a binary in the Cargo target directory.
/// Cargo sets CARGO_MANIFEST_DIR to the knowledge crate root.
/// The event-bus binary is in the sibling repo's target dir.
/// A sibling daemon binary, resolved beside this test binary.
///
/// Derived from `current_exe` (`<target>/debug/deps/<test>-<hash>`, so two levels
/// up is `<target>/debug`) rather than from the source layout. The previous form
/// walked up from `CARGO_MANIFEST_DIR` to `<repo>/daemons/<name>/target/debug/`,
/// which was the pre-monorepo shape where each daemon was its own repo with its
/// own target dir. Since the restructure the root `.cargo/config.toml` puts every
/// binary in one `target/`, so that path has not existed for a long time - and
/// because this test is `#[ignore]`d, nothing ever reported it.
fn binary_path(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("locate the test binary");
    exe.parent()
        .and_then(|deps| deps.parent())
        .expect("test binary lives in <target>/debug/deps")
        .join(name)
}

/// Wait until a Unix socket file exists, polling every 50ms.
/// Panics if the timeout is exceeded.
fn wait_for_socket(path: &str, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if std::path::Path::new(path).exists() {
            return;
        }
        assert!(
            start.elapsed() < timeout,
            "timed out waiting for socket: {path}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Send a single Event as a length-prefixed protobuf message over a Unix socket.
fn send_event(socket_path: &str, event: &proto::Event) {
    let encoded = event.encode_to_vec();
    let len = u32::try_from(encoded.len())
        .expect("event too large")
        .to_be_bytes();

    let mut stream = UnixStream::connect(socket_path)
        .unwrap_or_else(|e| panic!("failed to connect to producer socket {socket_path}: {e}"));

    stream.write_all(&len).expect("failed to write length");
    stream.write_all(&encoded).expect("failed to write event");
    stream.shutdown(Shutdown::Both).ok();
}

/// Helper that kills a child process when dropped.
/// This ensures cleanup even if the test panics.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

// L3 cross-daemon integration: spawns the sibling event-bus and knowledge
// binaries over real sockets. The per-crate test run does not build the
// event-bus binary, so this is excluded from the default suite and runs in
// the dedicated integration stage with `cargo test -- --ignored` after both
// daemons are built.
#[ignore = "L3 integration: needs event-bus + knowledge binaries prebuilt"]
#[tokio::test]
async fn event_lands_in_sqlite() {
    // Use a temporary directory so tests do not interfere with each other
    // or with a running system.
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let producer_socket = tmp.path().join("producer.sock");
    let consumer_socket = tmp.path().join("consumer.sock");
    let db_path = tmp.path().join("events.db");

    let producer_socket_str = producer_socket.to_str().unwrap();
    let consumer_socket_str = consumer_socket.to_str().unwrap();
    let db_path_str = db_path.to_str().unwrap();

    // Start the event-bus daemon.
    let _event_bus = KillOnDrop(
        Command::new(binary_path("event-bus"))
            .env("ARLEN_PRODUCER_SOCKET", producer_socket_str)
            .env("ARLEN_CONSUMER_SOCKET", consumer_socket_str)
            .env("RUST_LOG", "error") // suppress noise in test output
            .spawn()
            .expect("failed to start event-bus"),
    );

    // Wait for the event-bus sockets to appear.
    wait_for_socket(producer_socket_str, Duration::from_secs(5));
    wait_for_socket(consumer_socket_str, Duration::from_secs(5));

    // Start the knowledge daemon.
    let _knowledge = KillOnDrop(
        Command::new(binary_path("arlen-graph-daemon"))
            .env("ARLEN_CONSUMER_SOCKET", consumer_socket_str)
            .env("ARLEN_DB_PATH", db_path_str)
            .env("ARLEN_GRAPH_PATH", tmp.path().join("graph").to_str().unwrap())
            .env("ARLEN_DAEMON_SOCKET", tmp.path().join("daemon.sock").to_str().unwrap())
            .env("RUST_LOG", "error")
            .spawn()
            .expect("failed to start knowledge"),
    );

    // Give knowledge time to connect and register as a consumer.
    std::thread::sleep(Duration::from_millis(200));

    // Send a synthetic event.
    let event = proto::Event {
        id: "01950000-0000-7000-8000-000000000099".to_string(),
        r#type: "file.opened".to_string(),
        timestamp: 1_000_000,
        source: "test".to_string(),
        pid: 42,
        session_id: "session-integration-test".to_string(),
        payload: vec![],
        uid: 1000,
        project_id: String::new(),
    };

    // Emit until it lands, rather than sleeping a fixed time and hoping.
    //
    // The writer registers as a consumer concurrently with the rest of the daemon's
    // startup, and the bus drops an event that has no consumer at dispatch time, so
    // a single send after a fixed sleep races registration. The same fixed-sleep
    // shape was why the hermetic integration suite moved to this pattern. Each
    // attempt still allows the writer's 500ms batch timer to fire before looking.
    let pool = loop_until_open(db_path_str).await;

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut found: Option<(String, String)> = None;
    while std::time::Instant::now() < deadline {
        send_event(producer_socket_str, &event);
        std::thread::sleep(Duration::from_millis(800));
        if let Ok(row) = sqlx::query_as::<_, (String, String)>(
            "SELECT id, type FROM events WHERE id = ?",
        )
        .bind(&event.id)
        .fetch_one(&pool)
        .await
        {
            found = Some(row);
            break;
        }
    }

    let row = found.expect(
        "the event never reached SQLite: the producer -> bus -> writer -> store path \
         is broken, not merely slow (retried for 30s)",
    );
    assert_eq!(row.0, event.id);
    assert_eq!(row.1, event.r#type);
}

/// The daemon creates the database on startup, so opening it can lose a race with
/// the daemon's own first write. Retries until it opens or the deadline passes.
async fn loop_until_open(db_path: &str) -> sqlx::SqlitePool {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        match SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{db_path}"))
            .await
        {
            Ok(pool) => return pool,
            Err(e) if std::time::Instant::now() >= deadline => {
                panic!("the knowledge daemon never created its database: {e}")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}
