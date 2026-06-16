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
    /// yet; call [`spawn`](Self::spawn) for each one the scenario needs. The
    /// knowledge daemon's data + timeline-mount dirs are pre-created under the
    /// root so a spawned knowledge daemon is fully hermetic (writes its SQLite +
    /// graph under the temp root, never `/var/lib`).
    pub fn new() -> std::io::Result<Self> {
        let runtime = tempfile::Builder::new().prefix("arlen-it-").tempdir()?;
        std::fs::create_dir_all(runtime.path().join("knowledge"))?;
        std::fs::create_dir_all(runtime.path().join("timeline"))?;
        std::fs::create_dir_all(runtime.path().join("permissions"))?;
        // A private config home (`XDG_CONFIG_HOME`) so a spawned daemon reads no
        // real user config. Without it the knowledge daemon's project watcher
        // falls back to `default_watch_dirs` (`~/Repositories`, `~/Projects`, ...)
        // and scans the dev's REAL repositories: a hermeticity leak (spurious
        // Project nodes) and a needless cost. Seed an empty project watch list so
        // the watcher scans nothing by default; a scenario that wants detection
        // calls `seed_project_watch_dir` before spawning.
        std::fs::create_dir_all(runtime.path().join("config/arlen"))?;
        std::fs::write(
            runtime.path().join("config/arlen/graph.toml"),
            "[projects]\nwatch_directories = []\n",
        )?;
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

    /// The knowledge daemon's SQLite `events.db` path (matches `base_env`'s
    /// `ARLEN_DB_PATH`), for a scenario that asserts on the raw event store.
    pub fn db_path(&self) -> PathBuf {
        self.socket_path("knowledge/events.db")
    }

    /// The base environment every daemon inherits: the runtime root, the
    /// explicit socket overrides, AND the knowledge daemon's data + timeline
    /// paths, all pointed at this stack's temp dir, plus `XDG_RUNTIME_DIR` so
    /// daemons that derive `$XDG_RUNTIME_DIR/arlen` also land here. Setting the
    /// data paths is what makes a spawned knowledge daemon hermetic (it would
    /// otherwise write SQLite + the graph under `/var/lib`). A daemon that does
    /// not read a given var simply ignores it. Pure over the runtime path so it
    /// is testable without spawning.
    pub fn base_env(&self) -> BTreeMap<String, String> {
        let root = self.runtime.path().to_string_lossy().into_owned();
        let p = |rel: &str| self.runtime.path().join(rel).to_string_lossy().into_owned();
        let mut env = BTreeMap::from([
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
            // The knowledge daemon binds via ARLEN_DAEMON_SOCKET, but the ai-agent
            // connects via ARLEN_KNOWLEDGE_SOCKET (both default to the same
            // /run/arlen/knowledge.sock in production, so they agree there; they
            // only diverge under an override like this harness's). Set both names
            // to the one socket so either resolver finds it.
            (
                "ARLEN_KNOWLEDGE_SOCKET".to_string(),
                self.knowledge_socket().to_string_lossy().into_owned(),
            ),
            ("ARLEN_DB_PATH".to_string(), p("knowledge/events.db")),
            ("ARLEN_GRAPH_PATH".to_string(), p("knowledge/graph")),
            // Disable the timeline FUSE mount: the backend scenarios exercise the
            // event -> SQLite -> graph -> read path, not the `~/.timeline` view,
            // so skipping FUSE lets them run on a host (or CI runner) without
            // `/dev/fuse` while losing no coverage.
            ("ARLEN_TIMELINE_MOUNT".to_string(), "off".to_string()),
            // The daemon loads permission profiles from here (profile_path
            // checks ARLEN_PERMISSIONS_DIR first), so a profile seeded by
            // `seed_read_profile` is the one it reads for the caller.
            ("ARLEN_PERMISSIONS_DIR".to_string(), p("permissions")),
            // Private config home so a daemon reads only the seeded config (e.g.
            // the project watch list), never the real `~/.config/arlen`.
            ("XDG_CONFIG_HOME".to_string(), p("config")),
            // Private data home so a daemon that persists under `XDG_DATA_HOME`
            // (or `$HOME/.local/share`) writes under the temp root, not the real
            // user data dir. The audit daemon's HMAC key + ledger live here; the
            // daemon `create_dir_all`s `<data>/arlen` itself.
            ("XDG_DATA_HOME".to_string(), p("data")),
            // The ai-agent resolves `ai.toml` from `ARLEN_AI_CONFIG` (it reads
            // `$HOME/.config`, NOT XDG_CONFIG_HOME), so point it at the seeded
            // config under the private config home. Absent file -> the agent's
            // fail-closed defaults (AI off), so this is hermetic either way.
            ("ARLEN_AI_CONFIG".to_string(), p("config/arlen/ai.toml")),
            ("XDG_RUNTIME_DIR".to_string(), root),
        ]);
        // The audit daemon's ingest allowlist admits only the named AI-layer
        // producers (and their exact cargo-run dev ids); a test that submits
        // directly is NOT a producer, so name THIS test's own resolved dev id as
        // the daemon's one debug-only extra-admit (exact match, set only here).
        // Without it the audit-chain scenario's direct submit is refused.
        if let Some(id) = own_app_id() {
            env.insert("ARLEN_AUDIT_EXTRA_ADMIT".to_string(), id);
        }
        env
    }

    /// The audit daemon's read-API socket path (`$XDG_RUNTIME_DIR/arlen/audit-read.sock`).
    pub fn audit_read_socket(&self) -> PathBuf {
        self.runtime.path().join("arlen").join("audit-read.sock")
    }

    /// The audit daemon's ingest socket path (`$XDG_RUNTIME_DIR/arlen/audit-ingest.sock`).
    pub fn audit_ingest_socket(&self) -> PathBuf {
        self.runtime.path().join("arlen").join("audit-ingest.sock")
    }

    /// The private config home (`XDG_CONFIG_HOME` stand-in); a daemon's config
    /// (e.g. `arlen/graph.toml`, `arlen/ai.toml`) is read from here.
    pub fn config_home(&self) -> PathBuf {
        self.socket_path("config")
    }

    /// Point the knowledge daemon's project watcher at `dir` (rewriting the seeded
    /// `graph.toml` `[projects].watch_directories`), so a scenario can drive
    /// project detection from a controlled fixture directory. Must be called
    /// BEFORE spawning knowledge (the watcher loads its config at startup).
    pub fn seed_project_watch_dir(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::write(
            self.config_home().join("arlen/graph.toml"),
            format!(
                "[projects]\nwatch_directories = [\"{}\"]\n",
                dir.to_string_lossy()
            ),
        )
    }

    /// Seed a COMPLETE permission profile (with the mandatory `[info]` section)
    /// for `app_id`, granting `[graph].read` on `read_fields`. Unlike
    /// [`seed_profile_for`](Self::seed_profile_for) (a `[graph]`-only fragment the
    /// knowledge read-scope resolver tolerates), this is a full
    /// `arlen_permissions::PermissionProfile` that also parses under
    /// `ConnectionAuth` (the peer-auth path the audit daemon and other brokers
    /// use, which requires `[info]`). Needed for a principal that connects to a
    /// `ConnectionAuth`-gated socket, e.g. the agent submitting to the audit
    /// daemon. The caller's tier is still derived daemon-side from the quota
    /// config, so `tier` here only satisfies the profile schema.
    pub fn seed_full_profile_for(
        &self,
        app_id: &str,
        tier: &str,
        read_fields: &[&str],
    ) -> std::io::Result<()> {
        let reads = read_fields
            .iter()
            .map(|f| format!("    \"{f}\","))
            .collect::<Vec<_>>()
            .join("\n");
        let toml = format!(
            "[info]\napp_id = \"{app_id}\"\ntier = \"{tier}\"\n\n[graph]\nread = [\n{reads}\n]\n"
        );
        std::fs::write(self.permissions_dir().join(format!("{app_id}.toml")), toml)
    }

    /// Seed the agent's executor go-live profile: the exact `[graph]` grant the
    /// shipped `ai-agent.toml` carries for the auto-tag workflow — read scope on
    /// File/Project, the single `FILE_PART_OF` relation, and `instance_scope =
    /// "all"` (both endpoints are system-owned nodes the agent does not own, so
    /// linking them needs the privileged all-instances scope, or the write
    /// socket refuses the relation as unanchored). Used by the live-executor
    /// scenario so the dev agent (FirstParty in debug) can actually write the
    /// edge. Mirrors `seed_full_profile_for`'s `[info]` shape so it also parses
    /// under `ConnectionAuth` (the agent connects to the audit daemon too).
    pub fn seed_executor_profile_for(&self, app_id: &str, tier: &str) -> std::io::Result<()> {
        let toml = format!(
            "[info]\napp_id = \"{app_id}\"\ntier = \"{tier}\"\n\n\
             [graph]\nread = [\n\
             \x20   \"system.File.id\",\n\
             \x20   \"system.File.path\",\n\
             \x20   \"system.Project.id\",\n\
             \x20   \"system.Project.root_path\",\n\
             ]\n\
             relations = [\n\
             \x20   {{ from = \"system.File\", to = \"system.Project\", type = \"FILE_PART_OF\" }},\n\
             ]\n\
             instance_scope = \"all\"\n"
        );
        std::fs::write(self.permissions_dir().join(format!("{app_id}.toml")), toml)
    }

    /// Write the agent's `ai.toml` into the private config home (the path the
    /// ai-agent resolves via `XDG_CONFIG_HOME`), so a scenario can enable a
    /// behaviour, set the read tier, and pick the action mode. Must be called
    /// BEFORE spawning the agent (it reads the config at startup).
    pub fn seed_ai_config(&self, text: &str) -> std::io::Result<()> {
        std::fs::write(self.config_home().join("arlen/ai.toml"), text)
    }

    /// The directory the daemon loads permission profiles from (via
    /// `ARLEN_PERMISSIONS_DIR`).
    pub fn permissions_dir(&self) -> PathBuf {
        self.socket_path("permissions")
    }

    /// Seed a permission profile granting graph **read** on `read_fields` (e.g.
    /// `"system.File.id"`) for THIS test process's own app id, so a scenario can
    /// make authorised reads. The daemon resolves the connecting test process to
    /// the same app id (both use `path_to_app_id` over `/proc/<pid>/exe`), and
    /// loads this profile from [`permissions_dir`](Self::permissions_dir) to mint
    /// the caller's read scope. Returns the resolved app id. (A read-only grant
    /// needs no `relations`/`instance_scope`.)
    pub fn seed_read_profile(&self, read_fields: &[&str]) -> std::io::Result<String> {
        let app_id = own_app_id()
            .ok_or_else(|| std::io::Error::other("could not resolve own app id"))?;
        self.seed_profile_for(&app_id, read_fields)?;
        Ok(app_id)
    }

    /// Seed a `[graph].read` profile for an arbitrary `app_id` (not this process's
    /// own), so a scenario can act on another principal's profile, e.g. a revoke
    /// whose target is a different app. Writes `<permissions_dir>/{app_id}.toml`,
    /// the path the daemon resolves via `ARLEN_PERMISSIONS_DIR`.
    pub fn seed_profile_for(&self, app_id: &str, read_fields: &[&str]) -> std::io::Result<()> {
        let reads = read_fields
            .iter()
            .map(|f| format!("    \"{f}\","))
            .collect::<Vec<_>>()
            .join("\n");
        let toml = format!("[graph]\nread = [\n{reads}\n]\n");
        std::fs::write(self.permissions_dir().join(format!("{app_id}.toml")), toml)
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

    /// Like [`spawn`](Self::spawn) but redirects the child's stdout+stderr to
    /// `log_path` (an absolute path, typically outside the temp root so it
    /// survives teardown) instead of nulling them. For diagnosing a spawned
    /// daemon that produces no observable effect.
    pub fn spawn_logged(
        &mut self,
        repo: &str,
        bin: &str,
        extra_env: &[(&str, &str)],
        log_path: &Path,
    ) -> std::io::Result<()> {
        let path = binary_path(repo, bin);
        let log = std::fs::File::create(log_path)?;
        let log_err = log.try_clone()?;
        let mut cmd = Command::new(&path);
        for (k, v) in self.base_env() {
            cmd.env(k, v);
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log))
            .stderr(std::process::Stdio::from(log_err));
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
    repo_path(&format!("{repo}/target/debug/{name}"))
}

/// Resolve a path relative to the repo root (the integration crate's manifest
/// dir is `dev/integration`, so the root is its grandparent). Useful for locating
/// in-tree fixtures such as the agent behaviour directory.
pub fn repo_path(rel: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR set under cargo");
    PathBuf::from(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("dev/integration has a grandparent (repo root)")
        .join(rel)
}

/// Resolve THIS process's app id the same way the daemon resolves a connecting
/// peer: `path_to_app_id` over the real executable path (`/proc/self/exe`
/// readlinked). Both sides run the same resolver on the same binary, so the id
/// the test seeds a profile for is the id the daemon loads. `None` if the exe
/// link or the resolution fails. In a debug test binary this is `dev.<name>`
/// (the dev fallback rule).
pub fn own_app_id() -> Option<String> {
    let exe = std::fs::read_link("/proc/self/exe").ok()?;
    arlen_permissions::identity::path_to_app_id(&exe).ok()
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
        // The knowledge data paths land under the root too (hermetic: no
        // /var/lib or $HOME/.timeline writes).
        assert!(env["ARLEN_DB_PATH"].starts_with(&root));
        assert!(env["ARLEN_DB_PATH"].ends_with("knowledge/events.db"));
        assert!(env["ARLEN_GRAPH_PATH"].starts_with(&root));
        assert!(env["ARLEN_TIMELINE_MOUNT"].starts_with(&root));
        // The private config + data homes keep config/state reads hermetic.
        assert!(env["XDG_CONFIG_HOME"].starts_with(&root));
        assert!(env["XDG_DATA_HOME"].starts_with(&root));
        // The audit sockets resolve under the runtime root's arlen/ subdir.
        assert!(stack.audit_read_socket().starts_with(&root));
        assert!(stack.audit_ingest_socket().ends_with("arlen/audit-ingest.sock"));
    }

    #[test]
    fn new_precreates_the_knowledge_data_dirs() {
        let stack = EphemeralStack::new().unwrap();
        assert!(stack.runtime_dir().join("knowledge").is_dir());
        assert!(stack.runtime_dir().join("timeline").is_dir());
    }

    #[test]
    fn new_seeds_a_private_config_home_with_an_empty_project_watch_list() {
        // The hermeticity fix: a spawned daemon reads this config home, not the
        // real `~/.config/arlen`, and the seeded graph.toml scans no directories
        // (so the project watcher never touches the dev's real repos).
        let stack = EphemeralStack::new().unwrap();
        let env = stack.base_env();
        assert_eq!(env["XDG_CONFIG_HOME"], stack.config_home().to_string_lossy());
        let graph_toml = stack.config_home().join("arlen/graph.toml");
        let body = std::fs::read_to_string(&graph_toml).expect("seeded graph.toml");
        assert!(body.contains("watch_directories = []"), "got: {body}");
    }

    #[test]
    fn seed_project_watch_dir_points_the_watcher_at_a_fixture() {
        let stack = EphemeralStack::new().unwrap();
        let fixture = stack.runtime_dir().join("proj-fixture");
        stack.seed_project_watch_dir(&fixture).unwrap();
        let body =
            std::fs::read_to_string(stack.config_home().join("arlen/graph.toml")).unwrap();
        assert!(body.contains("proj-fixture"), "got: {body}");
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
    fn own_app_id_resolves_to_a_non_empty_id() {
        // The test binary lives under target/debug/deps, so the dev fallback
        // rule yields a `dev.`-prefixed id; we only assert it is resolvable and
        // non-empty (the exact name is the test binary's).
        let id = own_app_id().expect("own app id resolves");
        assert!(!id.is_empty());
    }

    #[test]
    fn seed_read_profile_writes_the_grant_for_the_caller() {
        let stack = EphemeralStack::new().unwrap();
        let app_id = stack
            .seed_read_profile(&["system.File.id", "system.File.path"])
            .expect("seed profile");
        let profile = stack.permissions_dir().join(format!("{app_id}.toml"));
        let body = std::fs::read_to_string(&profile).expect("profile written");
        assert!(body.contains("[graph]"));
        assert!(body.contains("\"system.File.id\""));
        assert!(body.contains("\"system.File.path\""));
        // The same id the daemon will resolve for the connecting peer.
        assert_eq!(app_id, own_app_id().unwrap());
    }

    #[test]
    fn base_env_points_profile_loading_at_the_temp_dir() {
        let stack = EphemeralStack::new().unwrap();
        let env = stack.base_env();
        assert_eq!(
            env["ARLEN_PERMISSIONS_DIR"],
            stack.permissions_dir().to_string_lossy()
        );
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
