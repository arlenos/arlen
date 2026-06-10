//! The hermetic ephemeral-stack harness (integration-testing-plan.md IT-0).
//!
//! Each integration scenario spawns its own daemons against a private runtime
//! root (a temp dir standing in for `/run/arlen/` and `$XDG_RUNTIME_DIR/arlen`),
//! waits on each daemon's readiness probe (the socket it binds), yields the live
//! socket paths to the test, and tears the whole stack down on drop. No
//! cross-scenario state leak: each [`EphemeralStack`] is fully isolated, so an
//! overnight run is interpretable rather than order-dependent flake.
//!
//! The daemons take their socket paths from the environment (`ARLEN_RUNTIME_DIR`
//! and the explicit `ARLEN_*_SOCKET` overrides, the same contract
//! `dev/process-compose.yaml` uses), so the harness points those at the temp
//! root before spawning. Binaries are located in each repo's `target/debug`
//! (built beforehand, like the existing `integration_compositor` test).
//!
//! The harness itself is synchronous (spawn + poll + kill); a scenario that
//! needs async (sqlx, a tokio socket client) drives it from a `#[tokio::test]`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// How long to wait for a daemon to bind its socket before failing the scenario.
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// A hermetic, ephemeral daemon stack rooted at a private runtime dir.
///
/// Spawn the daemons a scenario needs with [`spawn`](Self::spawn), wait for each
/// with [`wait_socket`](Self::wait_socket), read the socket paths via the
/// accessors, and let it drop to tear everything down.
pub struct EphemeralStack {
    /// The private runtime root (`/run/arlen/` + `$XDG_RUNTIME_DIR/arlen` stand-in).
    /// Dropped last, removing every socket and the seeded corpus.
    runtime: TempDir,
    /// Spawned daemons, killed on drop (reverse spawn order).
    children: Vec<Child>,
}

impl EphemeralStack {
    /// Create an empty stack with a fresh private runtime root. No daemon runs
    /// yet; call [`spawn`](Self::spawn) for each one the scenario needs.
    pub fn new() -> std::io::Result<Self> {
        let runtime = tempfile::Builder::new().prefix("arlen-it-").tempdir()?;
        Ok(Self {
            runtime,
            children: Vec::new(),
        })
    }

    /// The private runtime root (every socket lives directly under it).
    pub fn runtime_dir(&self) -> &Path {
        self.runtime.path()
    }

    /// The path a socket named `name` binds at under the runtime root (e.g.
    /// `event-bus-producer.sock`, `knowledge.sock`). Pure derivation; the socket
    /// need not exist yet.
    pub fn socket_path(&self, name: &str) -> PathBuf {
        self.runtime.path().join(name)
    }

    /// The event-bus producer socket path.
    pub fn producer_socket(&self) -> PathBuf {
        self.socket_path("event-bus-producer.sock")
    }

    /// The event-bus consumer socket path.
    pub fn consumer_socket(&self) -> PathBuf {
        self.socket_path("event-bus-consumer.sock")
    }

    /// The knowledge daemon query/write socket path.
    pub fn knowledge_socket(&self) -> PathBuf {
        self.socket_path("knowledge.sock")
    }

    /// The base environment every daemon inherits: the runtime root and the
    /// explicit socket overrides, all pointed at this stack's temp dir, plus
    /// `XDG_RUNTIME_DIR` so daemons that derive `$XDG_RUNTIME_DIR/arlen` also
    /// land here. Pure over the runtime path so it is testable without spawning.
    pub fn base_env(&self) -> BTreeMap<String, String> {
        let root = self.runtime.path().to_string_lossy().into_owned();
        BTreeMap::from([
            ("ARLEN_RUNTIME_DIR".to_string(), root.clone()),
            (
                "ARLEN_PRODUCER_SOCKET".to_string(),
                self.producer_socket().to_string_lossy().into_owned(),
            ),
            (
                "ARLEN_CONSUMER_SOCKET".to_string(),
                self.consumer_socket().to_string_lossy().into_owned(),
            ),
            (
                "ARLEN_DAEMON_SOCKET".to_string(),
                self.knowledge_socket().to_string_lossy().into_owned(),
            ),
            ("XDG_RUNTIME_DIR".to_string(), root),
        ])
    }

    /// Spawn a daemon binary (`<repo>/target/debug/<bin>`) with the base
    /// environment plus `extra_env`, its stdio nulled. The child is tracked and
    /// killed on drop. Does not wait for readiness; call
    /// [`wait_socket`](Self::wait_socket) after.
    pub fn spawn(
        &mut self,
        repo: &str,
        bin: &str,
        extra_env: &[(&str, &str)],
    ) -> std::io::Result<()> {
        let path = binary_path(repo, bin);
        let mut cmd = Command::new(&path);
        for (k, v) in self.base_env() {
            cmd.env(k, v);
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = cmd.spawn()?;
        self.children.push(child);
        Ok(())
    }

    /// Block until the socket named `name` appears under the runtime root, the
    /// readiness contract `process-compose.yaml` uses. Returns the socket path on
    /// success; errors if it does not appear within `timeout`.
    pub fn wait_socket(&self, name: &str, timeout: Duration) -> std::io::Result<PathBuf> {
        let path = self.socket_path(name);
        let start = Instant::now();
        loop {
            if path.exists() {
                return Ok(path);
            }
            if start.elapsed() >= timeout {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("daemon socket {name} never appeared within {timeout:?}"),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for EphemeralStack {
    fn drop(&mut self) {
        // Kill in reverse spawn order (consumers before producers). A daemon
        // that already exited just yields an error we ignore; the temp dir is
        // removed when `runtime` drops after this.
        for mut child in self.children.drain(..).rev() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Locate a binary in its repo's `target/debug` directory, relative to the
/// integration crate's manifest dir (the workspace root is its parent's parent:
/// `dev/integration` -> `dev` -> repo root). Matches the existing
/// `integration_compositor` test's resolution.
pub fn binary_path(repo: &str, name: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR set under cargo");
    let repo_root = PathBuf::from(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("dev/integration has a grandparent (repo root)")
        .to_path_buf();
    repo_root.join(repo).join("target").join("debug").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_paths_are_under_the_private_runtime_root() {
        let stack = EphemeralStack::new().unwrap();
        let root = stack.runtime_dir().to_path_buf();
        assert!(stack.producer_socket().starts_with(&root));
        assert!(stack.consumer_socket().starts_with(&root));
        assert!(stack.knowledge_socket().starts_with(&root));
        assert_eq!(stack.socket_path("x.sock"), root.join("x.sock"));
    }

    #[test]
    fn base_env_points_every_socket_at_the_runtime_root() {
        let stack = EphemeralStack::new().unwrap();
        let env = stack.base_env();
        let root = stack.runtime_dir().to_string_lossy().into_owned();
        assert_eq!(env["ARLEN_RUNTIME_DIR"], root);
        assert_eq!(env["XDG_RUNTIME_DIR"], root);
        assert!(env["ARLEN_PRODUCER_SOCKET"].starts_with(&root));
        assert!(env["ARLEN_DAEMON_SOCKET"].ends_with("knowledge.sock"));
    }

    #[test]
    fn two_stacks_get_distinct_runtime_roots() {
        // The isolation property: no two scenarios share a runtime root, so
        // there is no cross-scenario socket/corpus leak.
        let a = EphemeralStack::new().unwrap();
        let b = EphemeralStack::new().unwrap();
        assert_ne!(a.runtime_dir(), b.runtime_dir());
    }

    #[test]
    fn binary_path_resolves_under_the_repo_root() {
        let p = binary_path("daemons/event-bus", "event-bus");
        assert!(p.ends_with("daemons/event-bus/target/debug/event-bus"));
    }

    #[test]
    fn wait_socket_times_out_when_no_daemon_binds() {
        let stack = EphemeralStack::new().unwrap();
        let err = stack
            .wait_socket("never.sock", Duration::from_millis(120))
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn drop_removes_the_runtime_root() {
        let path = {
            let stack = EphemeralStack::new().unwrap();
            stack.runtime_dir().to_path_buf()
        };
        assert!(!path.exists(), "the private runtime root is removed on drop");
    }
}
