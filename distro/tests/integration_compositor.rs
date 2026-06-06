/// End-to-end integration test: Phase 1B exit criterion.
///
/// Starts event-bus, knowledge daemon, and compositor in nested mode.
/// Opens a minimal Wayland window using the test-client binary.
/// Verifies that a window.focused event lands in SQLite.
///
/// Requires all binaries to be built before running:
///   cargo build --manifest-path ../event-bus/Cargo.toml
///   cargo build --manifest-path ../knowledge/Cargo.toml
///   cargo build --manifest-path ../compositor/Cargo.toml
///
/// Run with:
///   cargo test --manifest-path distro/Cargo.toml --test integration_compositor
use sqlx::sqlite::SqlitePoolOptions;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

mod proto {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

/// Locate a binary in its repo's target/debug directory.
fn binary_path(repo: &str, name: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = PathBuf::from(&manifest_dir)
        .parent()
        .unwrap()
        .to_path_buf();
    workspace_root
        .join(repo)
        .join("target")
        .join("debug")
        .join(name)
}

/// Wait until a Unix socket file exists, polling every 50ms.
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

/// Wait for a new wayland-* socket to appear in XDG_RUNTIME_DIR.
/// Returns the display name (e.g. "wayland-2").
fn wait_for_new_wayland_socket(runtime_dir: &str, existing: &std::collections::HashSet<String>, timeout: Duration) -> String {
    let start = std::time::Instant::now();
    loop {
        assert!(start.elapsed() < timeout, "timed out waiting for compositor wayland socket");
        if let Ok(entries) = std::fs::read_dir(runtime_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("wayland-") && !name.ends_with(".lock") && !existing.contains(&name) {
                    return name;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

#[tokio::test]
async fn compositor_window_focused_lands_in_sqlite() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let producer_socket = tmp.path().join("producer.sock");
    let consumer_socket = tmp.path().join("consumer.sock");
    let db_path = tmp.path().join("events.db");
    let graph_path = tmp.path().join("graph");
    let daemon_socket = tmp.path().join("daemon.sock");

    let producer_str = producer_socket.to_str().unwrap();
    let consumer_str = consumer_socket.to_str().unwrap();
    let db_str = db_path.to_str().unwrap();
    let graph_str = graph_path.to_str().unwrap();
    let daemon_str = daemon_socket.to_str().unwrap();

    // Start event-bus
    let _event_bus = KillOnDrop(
        Command::new(binary_path("event-bus", "event-bus"))
            .env("LUNARIS_PRODUCER_SOCKET", producer_str)
            .env("LUNARIS_CONSUMER_SOCKET", consumer_str)
            .env("RUST_LOG", "error")
            .spawn()
            .expect("failed to start event-bus"),
    );

    wait_for_socket(producer_str, Duration::from_secs(5));
    wait_for_socket(consumer_str, Duration::from_secs(5));

    // Start knowledge daemon
    let _knowledge = KillOnDrop(
        Command::new(binary_path("knowledge", "knowledge"))
            .env("LUNARIS_CONSUMER_SOCKET", consumer_str)
            .env("LUNARIS_DB_PATH", db_str)
            .env("LUNARIS_GRAPH_PATH", graph_str)
            .env("LUNARIS_DAEMON_SOCKET", daemon_str)
            .env("RUST_LOG", "error")
            .spawn()
            .expect("failed to start knowledge"),
    );

    std::thread::sleep(Duration::from_millis(300));

    // Snapshot existing Wayland sockets BEFORE starting the compositor.
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/run/user/1000".to_string());
    let existing_sockets: std::collections::HashSet<String> = std::fs::read_dir(&runtime_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
        .collect();

    // Start compositor in nested mode.
    // DISPLAY must be explicitly passed so the X11 backend can connect.
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
    let _compositor = KillOnDrop(
        Command::new(binary_path("compositor", "cosmic-comp"))
            .env("LUNARIS_PRODUCER_SOCKET", producer_str)
            .env("DISPLAY", &display)
            .env("RUST_LOG", "error")
            .arg("--nested")
            .spawn()
            .expect("failed to start compositor"),
    );

    // Wait for the new Wayland socket to appear.
    eprintln!("DEBUG: existing sockets before compositor start: {:?}", existing_sockets);
    let compositor_display = wait_for_new_wayland_socket(&runtime_dir, &existing_sockets, Duration::from_secs(10));
    eprintln!("DEBUG: compositor socket found: {}", compositor_display);

    // Give compositor time to fully initialize
    std::thread::sleep(Duration::from_millis(500));

    // Open a minimal Wayland window using the test-client binary
    let mut client = Command::new(binary_path("compositor", "test-client"))
        .env("WAYLAND_DISPLAY", compositor_display)
        .env("RUST_LOG", "error")
        .spawn()
        .expect("failed to start test-client");

    // Wait for test-client to finish (it exits after 500ms)
    client.wait().expect("test-client failed");

    // Wait for knowledge daemon batch timer (500ms) plus margin
    std::thread::sleep(Duration::from_millis(800));

    // Check SQLite for window.focused event
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite:{db_str}"))
        .await
        .expect("failed to open db");

    let row: Option<(String,)> =
        sqlx::query_as("SELECT type FROM events WHERE type = 'window.focused' LIMIT 1")
            .fetch_optional(&pool)
            .await
            .expect("query failed");

    assert!(
        row.is_some(),
        "expected window.focused event in SQLite but found none"
    );
}
