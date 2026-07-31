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
//! root before spawning. Binaries are located in the shared
//! `<repo-root>/target/debug` (one target dir for every crate, set by the
//! repo-root `.cargo/config.toml`; built beforehand).
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
    /// When true (the default), `base_env` grants the resolved test caller the
    /// FirstParty (system-anchored) tier (`ARLEN_KNOWLEDGE_EXTRA_FIRST_PARTY`).
    ///
    /// This is on by default because a NON-root knowledge daemon (the harness runs
    /// every daemon as the developer uid) cannot read a same-uid peer's `/proc`:
    /// both the exe-based identity resolution AND the ThirdParty read-scope token
    /// mint (`issue_token_for_pid`, which re-reads `/proc/<pid>` for the PID-reuse
    /// guard) fail with EACCES. A system-anchored caller bypasses that per-request
    /// token mint, so it is the only tier a non-root daemon can serve a read to.
    /// The scoped-ThirdParty read path is covered by the daemon's own unit tests;
    /// the assembled-stack IT exercises the read as system-anchored. The deployed
    /// daemon runs as root and can do the ThirdParty mint directly.
    ///
    /// A scenario whose assertion needs an UNprivileged caller (the write-tier and
    /// authority-read refusals) calls [`as_unprivileged`](Self::as_unprivileged)
    /// before spawning knowledge to drop back to ThirdParty.
    first_party: bool,
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
            first_party: true,
        })
    }

    /// Drop the test caller to the unprivileged ThirdParty tier (no
    /// `ARLEN_KNOWLEDGE_EXTRA_FIRST_PARTY`) for every daemon spawned after this
    /// call. The caller is still resolved (via `ARLEN_KNOWLEDGE_DEV_SELF_ID`) so
    /// its identity is known; it just is not system-anchored. Needed by the
    /// scenarios whose assertion is a refusal of an unprivileged caller (the
    /// write-tier gate and the authority-label read gate). Call before spawning
    /// knowledge. See [`first_party`](Self::first_party) for why FirstParty is the
    /// default in a non-root harness.
    pub fn as_unprivileged(&mut self) -> &mut Self {
        self.first_party = false;
        self
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
        //
        // Same shape for the knowledge revoke op: it admits only `settings`
        // (and exact `dev.arlen-settings` in debug), so the revoke scenario's
        // direct call as the test's own dev id needs this debug-only exact
        // extra-admit.
        //
        // And the knowledge read-scope tier: the file manager's as-of
        // `FILE_PART_OF` traversal needs a FirstParty/system-anchored caller
        // (the rel-type token cannot be scoped per-label), so a seeded read
        // scenario sets the debug-only exact extra-first-party env to the test's
        // own dev id - the analog of the daemon's `dev.arlen-*` FirstParty admit.
        //
        // And the knowledge caller identity itself: this daemon runs non-root in
        // the harness, so it cannot read a same-uid peer's `/proc/<pid>/exe` and
        // would resolve THIS test connection to the `unknown` sentinel - which the
        // read-scope label gate denies regardless of the seeded profile or the
        // FirstParty tier above (both keyed on the resolved id). Declare the test's
        // own id so the debug-only daemon fallback resolves us to it; the deployed
        // root daemon reads the exe directly and never consults this.
        if let Some(id) = own_app_id() {
            env.insert("ARLEN_AUDIT_EXTRA_ADMIT".to_string(), id.clone());
            env.insert("ARLEN_REVOKE_EXTRA_ADMIT".to_string(), id.clone());
            env.insert("ARLEN_KNOWLEDGE_DEV_SELF_ID".to_string(), id.clone());
            // FirstParty tier is opt-in (see `first_party`): granting it globally
            // would make the deny scenarios' caller system-anchored and defeat
            // their refusal assertions.
            if self.first_party {
                env.insert("ARLEN_KNOWLEDGE_EXTRA_FIRST_PARTY".to_string(), id);
            }
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

    /// The consent broker's intake socket path
    /// (`$XDG_RUNTIME_DIR/arlen/consent-intake.sock`), where an app raises a
    /// consent request and blocks for the decision.
    pub fn consent_intake_socket(&self) -> PathBuf {
        self.runtime.path().join("arlen").join("consent-intake.sock")
    }

    /// The consent broker's control socket path
    /// (`$XDG_RUNTIME_DIR/arlen/consent-control.sock`), the trusted-shell side that
    /// fetches the front pending request and resolves it.
    pub fn consent_control_socket(&self) -> PathBuf {
        self.runtime.path().join("arlen").join("consent-control.sock")
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
        // A complete profile: `[info]` is required by the SDK `load_profile`
        // (which the daemon's token mint `issue_token_for_app` uses for the read
        // ops' readable-label scoping + the connect-time grant emission). A
        // `[graph]`-only fragment loads for the system-anchored read path but the
        // token mint rejects it, so a scoped ThirdParty caller's own grant /
        // provenance would never materialise. Mirrors a real shipped profile.
        let toml = format!("[info]\napp_id = \"{app_id}\"\n\n[graph]\nread = [\n{reads}\n]\n");
        std::fs::write(self.permissions_dir().join(format!("{app_id}.toml")), toml)
    }

    /// Seed an `[event_bus]` publish/subscribe scope for this process's own app
    /// id, so a scenario can exercise the bus under
    /// `ARLEN_EVENT_BUS_ENFORCE=1`. Returns the resolved app id.
    ///
    /// Both axes are needed even to test subscribe filtering: the bus scopes
    /// producers too, so a test that declared only `subscribe` would have its
    /// events dropped at publish and the consumer would see nothing for the
    /// wrong reason.
    ///
    /// Only System-tier peers (`/usr/bin/arlen-*`, `/usr/lib/arlen/*`) are exempt
    /// from these checks, and a test binary runs from `target/debug`, so it is
    /// held to this profile exactly like a third-party app - which is what makes
    /// it a faithful stand-in for one.
    pub fn seed_event_bus_profile(
        &self,
        publish: &[&str],
        subscribe: &[&str],
    ) -> std::io::Result<String> {
        let app_id = own_app_id()
            .ok_or_else(|| std::io::Error::other("could not resolve own app id"))?;
        let list = |v: &[&str]| {
            v.iter()
                .map(|p| format!("\"{p}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let toml = format!(
            "[info]\napp_id = \"{app_id}\"\n\n[event_bus]\npublish = [{}]\nsubscribe = [{}]\n",
            list(publish),
            list(subscribe),
        );
        std::fs::write(
            self.permissions_dir().join(format!("{app_id}.toml")),
            toml,
        )?;
        Ok(app_id)
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

    /// Start a private session D-Bus daemon under the runtime root and return its
    /// `unix:path=...` address. A minimal permissive config (own/send/receive
    /// allowed) lets a spawned daemon claim a name and the test call it. Tracked
    /// and killed on drop like any spawned child; after this,
    /// `wait_socket("dbus-session.sock", ...)` until it binds. `dbus-daemon` must
    /// be on PATH.
    ///
    /// **No scenario calls this.** It was written for the go-live undo
    /// rehearsal, where the agent registers `org.arlen.AIAgent1` and the test
    /// drives `completed_actions` -> `compensate`, and the native `arlen-ai-agent`
    /// that owned that name is retired: the two scenarios that spawned it are
    /// `#[ignore]`d as obsolete pending a pi-engine-daemon rewrite. So this is
    /// waiting for a bus-owning daemon to test rather than for someone to get
    /// round to it, which is a different kind of unused and worth saying, since
    /// the previous version of this comment read as pending work.
    pub fn start_session_bus(&mut self) -> std::io::Result<String> {
        let sock = self.runtime.path().join("dbus-session.sock");
        let addr = format!("unix:path={}", sock.to_string_lossy());
        let cfg = self.runtime.path().join("dbus-session.conf");
        std::fs::write(
            &cfg,
            format!(
                "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \
                 \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
                 <busconfig>\n\
                 \x20 <type>session</type>\n\
                 \x20 <listen>{addr}</listen>\n\
                 \x20 <policy context=\"default\">\n\
                 \x20   <allow own=\"*\"/>\n\
                 \x20   <allow send_destination=\"*\"/>\n\
                 \x20   <allow receive_sender=\"*\"/>\n\
                 \x20 </policy>\n\
                 </busconfig>\n"
            ),
        )?;
        let child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", cfg.to_string_lossy()))
            .arg("--nofork")
            .arg("--nopidfile")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        self.children.push(child);
        Ok(addr)
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

/// Locate a spawned binary. Every crate builds into the ONE shared target dir
/// (repo-root `.cargo/config.toml` `[build] target-dir = "target"`), so all
/// binaries land in `<repo-root>/target/debug/<name>` regardless of their crate.
/// `repo` is kept for the `spawn`/`binary_built` call-site API (and reads as the
/// producing crate) but no longer part of the artifact path.
pub fn binary_path(_repo: &str, name: &str) -> PathBuf {
    repo_path(&format!("target/debug/{name}"))
}

/// Whether a daemon binary has been built (its `target/debug/<name>` exists).
/// Scenarios that spawn a daemon the fast `just integration-smoke` does not
/// build (the audit daemon, the ai-agent) use this to skip gracefully with a
/// logged note, so the smoke run passes on every change while the full set runs
/// under `just integration-nightly` (which builds those daemons).
pub fn binary_built(repo: &str, name: &str) -> bool {
    binary_path(repo, name).exists()
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
        // The FUSE timeline mount is disabled in the harness ("off"), so a
        // non-FUSE scenario stays hermetic without needing a FUSE host; the few
        // FUSE scenarios opt in. (Was asserting a path under the root, stale since
        // base_env switched to the "off" sentinel.)
        assert_eq!(env["ARLEN_TIMELINE_MOUNT"], "off");
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
        assert!(p.ends_with("target/debug/event-bus"));
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

#[cfg(test)]
mod unit_identity {
    use super::repo_path;
    use arlen_permissions::identity::path_to_app_id;
    use std::path::{Path, PathBuf};

    /// Every shipped unit's `ExecStart`, as the deployment actually declares it.
    fn shipped_exec_starts() -> Vec<(PathBuf, String)> {
        let mut found = Vec::new();
        for root in ["daemons", "apps"] {
            collect(&repo_path(root), &mut found);
        }
        found
    }

    fn collect(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target" || n == "node_modules") {
                    continue;
                }
                collect(&p, out);
            } else if p.extension().is_some_and(|x| x == "service") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    for line in text.lines() {
                        if let Some(rest) = line.strip_prefix("ExecStart=") {
                            let bin = rest.split_whitespace().next().unwrap_or("").to_string();
                            if !bin.is_empty() {
                                out.push((p.clone(), bin));
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    /// A daemon that cannot be resolved from the path its own unit execs has no
    /// identity at deployment: every peer-authenticated socket it speaks to sees
    /// `UnknownBinary` and refuses it, and a fail-closed audit turns that into a
    /// dead feature. Three daemons have landed in this hole - the transfer daemon
    /// (live), the module runtime (latent), the agent before it - each time
    /// because a binary sits in libexec while the resolver kept a `/usr/bin`
    /// assumption that a hand-written test happily confirmed.
    ///
    /// Deriving the check from the shipped units closes it: the unit is the
    /// deployment's own statement of where the binary goes, so this cannot drift
    /// from reality the way a hand-picked path can.
    #[test]
    fn every_shipped_unit_binary_has_an_identity() {
        let mut unresolved = Vec::new();
        for (unit, bin) in shipped_exec_starts() {
            // Only Arlen's own binaries carry an app id; a unit that execs a
            // system tool (busctl, sh) is not making an identity claim.
            if !bin.starts_with("/usr/lib/arlen/") && !bin.starts_with("/usr/bin/arlen-") {
                continue;
            }
            if path_to_app_id(Path::new(&bin)).is_err() {
                let name = unit.file_name().unwrap().to_string_lossy().to_string();
                unresolved.push(format!("{name} execs {bin}"));
            }
        }
        assert!(
            unresolved.is_empty(),
            "shipped units whose binary has no app id ({}):\n  {}",
            unresolved.len(),
            unresolved.join("\n  ")
        );
    }
}

#[cfg(test)]
mod image_staging {
    use super::repo_path;
    use std::path::{Path, PathBuf};

    /// Units the IMAGE actually stages (not every `dist/*.service` in the tree -
    /// several daemons deliberately are not in the image yet).
    fn staged_units() -> Vec<PathBuf> {
        let mut out = Vec::new();
        collect(&repo_path("dev/mkosi/mkosi.extra/usr/lib/systemd"), &mut out);
        out
    }

    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().is_some_and(|x| x == "service") && !p.is_symlink() {
                out.push(p);
            }
        }
    }

    fn exec_start(unit: &Path) -> Option<String> {
        let text = std::fs::read_to_string(unit).ok()?;
        text.lines()
            .find_map(|l| l.strip_prefix("ExecStart="))
            .map(|rest| rest.split_whitespace().next().unwrap_or("").to_string())
            .filter(|s| !s.is_empty())
    }

    /// A staged unit whose binary nothing produces fails at boot with 203/EXEC,
    /// and every unit ordered after it starts anyway into a system missing that
    /// component. There are TWO ways a binary gets into the image - a
    /// `mkosi.build.d/*.sh.chroot` script, or `build-image.sh` staging a
    /// cross-built binary into the overlay - and checking only one of them
    /// produces a confident false positive (I managed exactly that on the event
    /// bus, and nearly wrote a redundant build script for it). So this checks
    /// both, plus a binary already sitting in the overlay.
    #[test]
    fn every_staged_unit_has_something_that_builds_its_binary() {
        let build_d = repo_path("dev/mkosi/mkosi.build.d");
        let scripts: String = std::fs::read_dir(&build_d)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .collect();
        let image_script =
            std::fs::read_to_string(repo_path("dev/mkosi/build-image.sh")).unwrap_or_default();
        let extra = repo_path("dev/mkosi/mkosi.extra");

        let mut missing = Vec::new();
        for unit in staged_units() {
            let Some(bin) = exec_start(&unit) else { continue };
            let name = Path::new(&bin).file_name().unwrap_or_default().to_string_lossy().to_string();
            let staged_file = extra.join(bin.trim_start_matches('/'));
            if scripts.contains(&name) || image_script.contains(&name) || staged_file.exists() {
                continue;
            }
            missing.push(format!(
                "{} execs {bin}, which no build script produces",
                unit.file_name().unwrap().to_string_lossy()
            ));
        }
        assert!(
            missing.is_empty(),
            "staged units whose binary is never built ({}):\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }
}

#[cfg(test)]
mod proto_agreement {
    use super::repo_path;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// Every copy of the event wire contract in the tree.
    fn proto_copies() -> Vec<PathBuf> {
        let mut out = Vec::new();
        for root in ["daemons", "sdk", "contracts", "apps", "dev"] {
            collect(&repo_path(root), &mut out);
        }
        out
    }

    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| {
                    n == "target" || n == "node_modules" || n.to_string_lossy().starts_with("mkosi")
                }) {
                    continue;
                }
                collect(&p, out);
            } else if p.file_name().is_some_and(|n| n == "event.proto") {
                out.push(p);
            }
        }
    }

    /// `(message, field) -> number`, parsed line-wise. The files are plain
    /// proto3 with one field per line, so a full parser would be more machinery
    /// than the check is worth; anything this misses simply is not compared.
    fn fields(path: &Path) -> BTreeMap<(String, String), u32> {
        let mut out = BTreeMap::new();
        let Ok(text) = std::fs::read_to_string(path) else {
            return out;
        };
        let mut message = String::new();
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("message ") {
                message = rest.trim_end_matches(" {").trim().to_string();
                continue;
            }
            if line == "}" {
                message.clear();
                continue;
            }
            if message.is_empty() {
                continue;
            }
            // `<type> <name> = <number>;`
            if let Some((decl, num)) = line.trim_end_matches(';').split_once('=') {
                let name = decl.split_whitespace().last().unwrap_or("");
                if let Ok(n) = num.trim().parse::<u32>() {
                    if !name.is_empty() {
                        out.insert((message.clone(), name.to_string()), n);
                    }
                }
            }
        }
        out
    }

    /// The event schema is COPIED into each daemon that speaks it rather than
    /// shared as one crate. Copies may legitimately differ by ABSENCE - a daemon
    /// that does not care about a field simply does not carry it, and proto3
    /// decodes an absent field to its default. What they may never do is
    /// disagree on a field NUMBER, because the number is the wire identity: two
    /// daemons would then read the same bytes as different fields, silently.
    ///
    /// Nothing checked this. I diffed the copies by hand once and found them
    /// consistent, which is a fact about that afternoon rather than a property
    /// of the tree.
    #[test]
    fn every_copy_of_the_event_schema_agrees_on_field_numbers() {
        let copies = proto_copies();
        assert!(copies.len() > 1, "expected several copies, found {}", copies.len());

        let mut seen: BTreeMap<(String, String), (u32, PathBuf)> = BTreeMap::new();
        let mut conflicts = Vec::new();
        for path in &copies {
            for (key, num) in fields(path) {
                match seen.get(&key) {
                    Some((first, first_path)) if *first != num => conflicts.push(format!(
                        "{}.{} is {} in {} but {} in {}",
                        key.0,
                        key.1,
                        first,
                        first_path.display(),
                        num,
                        path.display()
                    )),
                    Some(_) => {}
                    None => {
                        seen.insert(key, (num, path.clone()));
                    }
                }
            }
        }
        assert!(
            conflicts.is_empty(),
            "event schema copies disagree on field numbers ({}):\n  {}",
            conflicts.len(),
            conflicts.join("\n  ")
        );
    }
}

#[cfg(test)]
mod dbus_activation {
    use super::repo_path;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn key(text: &str, k: &str) -> Option<String> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("{k}=")))
            .map(|v| v.split_whitespace().next().unwrap_or("").to_string())
            .filter(|v| !v.is_empty())
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| {
                    n == "target" || n == "node_modules" || n.to_string_lossy().starts_with("mkosi")
                }) {
                    continue;
                }
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "service") {
                out.push(p);
            }
        }
    }

    /// A D-Bus activation file and the systemd unit it points at are two files
    /// that must agree on three things, with nothing checking any of them:
    ///
    /// - the `SystemdService=` must name a unit that exists, or activation starts
    ///   the bare `Exec=` binary directly and BYPASSES the unit's hardening (that
    ///   exact bug was found on `InstallDaemon1`, which had no `SystemdService=`
    ///   at all and would have run installd unsandboxed);
    /// - the unit's `BusName=` must be the name being activated, or the unit
    ///   never reports ready and `Type=dbus` hangs until timeout;
    /// - `Exec=` must be the unit's `ExecStart`, or the two disagree about which
    ///   binary serves the name.
    #[test]
    fn every_dbus_activation_file_agrees_with_its_unit() {
        let mut all = Vec::new();
        for root in ["daemons", "apps"] {
            walk(&repo_path(root), &mut all);
        }
        // Units by file name, so an activation file can find the one it names.
        let units: BTreeMap<String, PathBuf> = all
            .iter()
            .map(|p| (p.file_name().unwrap().to_string_lossy().to_string(), p.clone()))
            .collect();

        let activation: Vec<&PathBuf> = all
            .iter()
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("org."))
            .collect();
        assert!(!activation.is_empty(), "expected D-Bus activation files");

        let mut problems = Vec::new();
        for act in activation {
            let text = std::fs::read_to_string(act).unwrap_or_default();
            let name = act.file_name().unwrap().to_string_lossy().to_string();
            let Some(unit_name) = key(&text, "SystemdService") else {
                problems.push(format!("{name} has no SystemdService=, so activation bypasses the unit"));
                continue;
            };
            let Some(unit_path) = units.get(&unit_name) else {
                problems.push(format!("{name} names {unit_name}, which does not exist"));
                continue;
            };
            let unit = std::fs::read_to_string(unit_path).unwrap_or_default();
            if let (Some(bus), Some(declared)) = (key(&text, "Name"), key(&unit, "BusName")) {
                if bus != declared {
                    problems.push(format!(
                        "{name} activates {bus} but {unit_name} declares BusName={declared}"
                    ));
                }
            }
            if let (Some(exec), Some(start)) = (key(&text, "Exec"), key(&unit, "ExecStart")) {
                if exec != start {
                    problems.push(format!(
                        "{name} execs {exec} but {unit_name} starts {start}"
                    ));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "D-Bus activation files disagreeing with their units ({}):\n  {}",
            problems.len(),
            problems.join("\n  ")
        );
    }
}

#[cfg(test)]
mod ci_matrix {
    use super::repo_path;
    use std::path::{Path, PathBuf};

    /// The crate directories `ci.yml` builds, parsed from its `RUST_ALL` line.
    fn matrix() -> Vec<String> {
        let ci = std::fs::read_to_string(repo_path(".github/workflows/ci.yml"))
            .expect("read ci.yml");
        let line = ci
            .lines()
            .find(|l| l.trim_start().starts_with("RUST_ALL="))
            .expect("ci.yml declares RUST_ALL");
        // The value is a JSON array in single quotes: take what is between the
        // brackets and read the quoted entries out of it.
        let inner = line
            .split_once('[')
            .and_then(|(_, rest)| rest.rsplit_once(']'))
            .map(|(inner, _)| inner)
            .expect("RUST_ALL is a bracketed list");
        inner
            .split(',')
            .map(|e| e.trim().trim_matches('"').to_string())
            .filter(|e| !e.is_empty())
            .collect()
    }

    /// Every directory in the tree holding a `[package]` manifest.
    fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                if n == "target" || n == "node_modules" || n.starts_with("mkosi") || n == ".git" {
                    continue;
                }
                packages(&p, out);
            } else if p.file_name().is_some_and(|n| n == "Cargo.toml")
                && std::fs::read_to_string(&p).is_ok_and(|t| t.contains("[package]"))
            {
                out.push(p.parent().unwrap().to_path_buf());
            }
        }
    }

    /// `ci.yml` states the rule in its own comment: "Everything else that defines
    /// a `[package]` IS listed", with two documented exclusions - `apps/*/src-tauri`
    /// (needs webkit2gtk and the tauri toolchain) and `daemons/kernel-layer/*`
    /// (needs the bpf toolchain or a VM). Nothing enforced that, and it has drifted
    /// before: `dev/integration` sat outside the matrix for a while, so its tests
    /// never ran on a pull request despite being written to gate exactly that.
    ///
    /// A crate counts as covered if it is listed OR an ancestor is - the matrix
    /// names workspace roots like `sdk` and `ai` rather than each member.
    #[test]
    fn every_package_is_in_the_ci_matrix_or_documented_as_excluded() {
        let root = repo_path("");
        let listed = matrix();
        let mut found = Vec::new();
        packages(&root, &mut found);
        assert!(found.len() > 50, "expected to find the tree's crates, got {}", found.len());

        let mut missing = Vec::new();
        for dir in found {
            let rel = dir.strip_prefix(&root).unwrap_or(&dir).to_string_lossy().to_string();
            // The two exclusions ci.yml documents.
            if rel.contains("/src-tauri") || rel.starts_with("daemons/kernel-layer") {
                continue;
            }
            // Listed outright, or a member of a listed workspace. A crate that
            // declares its own `[workspace]` is NOT a member of anything above
            // it - `forage/patch` sits under the listed `forage` but is built
            // separately - so a shared path prefix is not coverage.
            let standalone = std::fs::read_to_string(dir.join("Cargo.toml"))
                .is_ok_and(|t| t.contains("[workspace]"));
            let covered = listed.iter().any(|l| {
                rel == *l || (!standalone && rel.starts_with(&format!("{l}/")))
            });
            if !covered {
                missing.push(rel);
            }
        }
        missing.sort();
        assert!(
            missing.is_empty(),
            "crates outside the CI matrix and outside its documented exclusions ({}):\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }
}

#[cfg(test)]
mod frontend_matrix {
    use super::repo_path;
    use std::path::{Path, PathBuf};

    fn front_all() -> Vec<String> {
        let ci = std::fs::read_to_string(repo_path(".github/workflows/ci.yml")).expect("ci.yml");
        let line = ci
            .lines()
            .find(|l| l.trim_start().starts_with("FRONT_ALL="))
            .expect("ci.yml declares FRONT_ALL");
        let inner = line
            .split_once('[')
            .and_then(|(_, r)| r.rsplit_once(']'))
            .map(|(i, _)| i)
            .expect("FRONT_ALL is a bracketed list");
        inner
            .split(',')
            .map(|e| e.trim().trim_matches('"').to_string())
            .filter(|e| !e.is_empty())
            .collect()
    }

    fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                if n == "node_modules" || n == "target" || n.starts_with("mkosi") || n == ".git" {
                    continue;
                }
                packages(&p, out);
            } else if p.file_name().is_some_and(|n| n == "package.json") {
                out.push(p);
            }
        }
    }

    /// `ci.yml` applies the same rule to `FRONT_ALL` as to `RUST_ALL`. The
    /// discriminating fact for a frontend package is whether it declares a
    /// `check` script: if it does, CI can type-check it, and a package that can
    /// be checked but is not listed simply never is. `apps/knowledge` and the
    /// portal's `picker-ui` were in exactly that position - both pass `npm run
    /// check` today, and neither had ever been run by CI.
    ///
    /// Packages with no `check` script (the `sdk/tauri-plugin-*` binding crates,
    /// which only build) are outside this by their own shape rather than by a
    /// list someone maintains.
    #[test]
    fn every_checkable_frontend_package_is_in_the_matrix() {
        let root = repo_path("");
        let listed = front_all();
        let mut found = Vec::new();
        packages(&root, &mut found);

        let mut missing = Vec::new();
        for manifest in found {
            let text = std::fs::read_to_string(&manifest).unwrap_or_default();
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if json.get("scripts").and_then(|s| s.get("check")).is_none() {
                continue;
            }
            let dir = manifest.parent().unwrap();
            let rel = dir.strip_prefix(&root).unwrap_or(dir).to_string_lossy().to_string();
            if !listed.iter().any(|l| rel == *l) {
                missing.push(rel);
            }
        }
        missing.sort();
        assert!(
            missing.is_empty(),
            "frontend packages with a `check` script that CI never runs ({}):\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }
}

#[cfg(test)]
mod module_reachability {
    use super::repo_path;
    use std::path::{Path, PathBuf};

    /// Modules declared in a daemon crate root that nothing in the tree reaches.
    ///
    /// Each is real, each compiles, each has passing unit tests, and none of them
    /// runs. They are listed rather than fixed because the fix is a design call:
    /// knowledge's `lifecycle` and `backup` need someone to decide when an
    /// uninstall or an export reaches the graph, and `sentinel-detect` is the pure
    /// detector core for a `org.arlen.Sentinel1` daemon that does not exist yet.
    ///
    /// The point of the list is that it cannot grow silently. Removing an entry
    /// once it is wired up is the expected direction of travel.
    const KNOWN_UNREACHED: &[&str] = &[
        // BR-4's decision core, both halves: `retry` says when a failed bridge
        // is tried again and when it must not be, `auth` when a credential must
        // be renewed. Neither is reachable until the sink reports typed errors
        // instead of `String`, because classifying transient against hard by
        // matching error text is the fragility they exist to avoid. That
        // contract change is the wiring step.
        "daemons/bridge-ingest/auth",
        "daemons/bridge-ingest/retry",
        // Surfaced once the matcher stopped taking another crate's module name
        // as evidence. Each is real work waiting on the piece that would call
        // it, named here rather than deleted, because deleting a tested core to
        // shorten a list is the wrong direction:
        //   code-indexer/resolve    CG-R2 query-time cross-file resolution; the
        //                           extractor records reference names and
        //                           nothing yet asks this to bind them.
        //   connections/revocation  CONN-R2 exit and expiry revocation of
        //                           derived tokens; no process-exit watcher
        //                           calls it.
        //   integration-packages/manifest  IP-R5's manifest, parsed by nothing
        //                           until the installer path reaches it.
        //   sentinel-detect/tracker the finder-tag classifier for a
        //                           `org.arlen.Sentinel1` daemon that does not
        //                           exist, like its siblings below.
        "daemons/code-indexer/resolve",
        "daemons/connections/revocation",
        "daemons/integration-packages/manifest",
        "daemons/sentinel-detect/tracker",
        // Diagnosed rather than assumed: the transfer daemon's live per-uid
        // listeners are deliberately deferred to PR-R1's per-uid sockets, which
        // `main.rs` states while holding a fail-closed `DeniedBroker` in the
        // meantime. These are that listener's socket-path and source-attestation
        // helpers. It only started failing this test when the module was renamed
        // off `dbus`, a name common enough that another crate's `dbus::` had
        // been standing in as its caller.
        "daemons/transfer-daemon/request_socket",
        "daemons/knowledge/backup",
        "daemons/knowledge/lifecycle",
        "daemons/knowledge/migration",
        "daemons/sentinel-detect/exposure",
        "daemons/sentinel-detect/movement",
        "daemons/sentinel-detect/recording",
        "daemons/sentinel-detect/usb",
    ];

    /// Every `.rs` file in the tree, skipping build and vendor directories.
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                if n == "target" || n == "node_modules" || n.starts_with("mkosi") || n == ".git" {
                    continue;
                }
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// Whether `text` names `needle` (a `module::` path) as a whole identifier.
    ///
    /// A plain substring search is wrong here and said so on the first run: it
    /// reported knowledge's `lifecycle` as reached because the xdg-portal daemon
    /// writes `picker_lifecycle::`, which contains it. The preceding character
    /// must not be part of an identifier for the match to be this module.
    fn mentions(text: &str, needle: &str) -> bool {
        text.match_indices(needle).any(|(i, _)| {
            i == 0
                || !text[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    }

    /// A crate's published name from its `Cargo.toml`, which is what an external
    /// caller writes in a path and is routinely not the directory name.
    fn package_name(manifest: &Path) -> Option<String> {
        let text = std::fs::read_to_string(manifest).ok()?;
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("name") {
                if let Some(v) = rest.trim_start().strip_prefix('=') {
                    return Some(v.trim().trim_matches('"').to_string());
                }
            }
            if line.starts_with('[') && line != "[package]" {
                break;
            }
        }
        None
    }

    /// The module names a crate root declares, as `mod x;` or `pub mod x;`.
    fn declared(root: &Path) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(root) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("pub mod ").or_else(|| l.strip_prefix("mod "))?;
                let name = rest.strip_suffix(';')?;
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    .then(|| name.to_string())
            })
            .collect()
    }

    /// A module declared in a daemon crate root but named by no file outside its
    /// own is dead: nothing can call it, and nothing ever will until someone
    /// wires it up. This found `knowledge/lifecycle`, 644 lines of entity trash,
    /// restore and staged uninstall that has never run, and `knowledge/backup`,
    /// 858 lines that are the only path by which a user would export their graph.
    ///
    /// Scoped to `daemons/*` on purpose. An `sdk/*` module can legitimately have
    /// no in-tree caller because a consumer outside this repo reaches it, and the
    /// compositor does exactly that for `arlen-theme`. Daemon crates have no such
    /// consumer: the compositor's only arlen dependency is `arlen-theme`, so for
    /// `daemons/*` "unreferenced in this tree" means unreferenced anywhere.
    ///
    /// The check is per module, so it does not see an entire crate going unused -
    /// `sentinel-detect`'s modules reference each other, which is why only four of
    /// its six appear above while the crate as a whole has no consumer at all.
    #[test]
    fn no_new_daemon_module_is_unreachable() {
        let root = repo_path("");
        let mut all = Vec::new();
        sources(&root, &mut all);
        assert!(all.len() > 500, "expected the tree's sources, got {}", all.len());

        let daemons = root.join("daemons");
        let mut crates: Vec<PathBuf> = Vec::new();
        for e in std::fs::read_dir(&daemons).expect("read daemons/").flatten() {
            if e.path().is_dir() {
                crates.push(e.path());
            }
        }
        crates.sort();

        let mut unreached = Vec::new();
        for c in crates {
            let rel_crate = c
                .strip_prefix(&root)
                .unwrap_or(&c)
                .to_string_lossy()
                .to_string();
            for root_file in ["src/lib.rs", "src/main.rs"] {
                for m in declared(&c.join(root_file)) {
                    // Its own file and its own directory do not count as callers.
                    let own_file = c.join(format!("src/{m}.rs"));
                    let own_dir = c.join("src").join(&m);
                    // Reached means something PATHS INTO it, and the path is
                    // scoped. Inside its own crate a bare `m::` is that path;
                    // from outside it has to be the crate's published name.
                    // Searching the whole tree for a bare `m::` was too loose:
                    // several daemons have a module called `auth`, and any one
                    // of them saying `auth::` made every other crate's look
                    // reached. The published name matters and is routinely not
                    // the directory - `daemons/settings-broker` publishes
                    // `arlen_settings_broker` - so deriving it from the folder
                    // reports modules thatexternal callers use every day as dead.
                    let inside = format!("{m}::");
                    let crate_name = package_name(&c.join("Cargo.toml"))
                        .unwrap_or_else(|| {
                            c.file_name().unwrap_or_default().to_string_lossy().to_string()
                        })
                        .replace('-', "_");
                    let outside = format!("{crate_name}::{m}::");
                    let reached = all.iter().any(|f| {
                        if *f == own_file || f.starts_with(&own_dir) {
                            return false;
                        }
                        let own_crate = f.starts_with(&c);
                        std::fs::read_to_string(f).is_ok_and(|t| {
                            (own_crate && mentions(&t, &inside)) || mentions(&t, &outside)
                        })
                    });
                    let id = format!("{rel_crate}/{m}");
                    if !reached && !unreached.contains(&id) {
                        unreached.push(id);
                    }
                }
            }
        }
        unreached.sort();

        let known: Vec<String> = KNOWN_UNREACHED.iter().map(|s| s.to_string()).collect();
        let fresh: Vec<&String> = unreached.iter().filter(|u| !known.contains(u)).collect();
        assert!(
            fresh.is_empty(),
            "daemon modules that compile and test but nothing reaches ({}):\n  {}\n\
             Wire it up, or add it to KNOWN_UNREACHED with the reason it waits.",
            fresh.len(),
            fresh.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  ")
        );

        let stale: Vec<&String> = known.iter().filter(|k| !unreached.contains(k)).collect();
        assert!(
            stale.is_empty(),
            "KNOWN_UNREACHED lists modules that are now reached ({}):\n  {}\n\
             Delete these entries: the list is meant to shrink.",
            stale.len(),
            stale.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  ")
        );
    }
}

#[cfg(test)]
mod crate_reachability {
    use super::repo_path;
    use std::path::{Path, PathBuf};

    /// Library crates nothing in the tree depends on.
    ///
    /// The module check above cannot see these, because a crate's own modules
    /// reference each other perfectly well while the crate as a whole has no
    /// consumer. Several are here because a successor took the job under a
    /// similar name, which is what makes them easy to miss when reading the tree.
    /// Each entry pairs the crate with WHY it waits, because the reason is the
    /// part that goes wrong. `sdk/monitor-reads` sat here under a neighbour's
    /// comment claiming two other crates did its work; they did not, and the
    /// entry read as permission to delete reads that exist nowhere else. A bare
    /// path inherits whatever comment happens to sit above it, so the reason is
    /// a field rather than a convention - unwritable without being written.
    const KNOWN_UNCONSUMED: &[(&str, &str)] = &[
        ("sdk/proc-collect",
         "Superseded, genuinely: `apps/system-monitor/core`'s `procmon` reads the process list, CPU and memory through `system-monitor-mcp`'s sysinfo, which is this crate's whole job."),
        ("sdk/config",
         "`sdk/config-format` and `daemons/config-broker` do this work, and the compositor parses its own keybindings. The one mention left is a commented-out dependency in `apps/settings/src-tauri/Cargo.toml` pointing at `github.com/arlenos/sdk`, a repo from before the monorepo."),
        ("sdk/tauri-plugin-clipboard",
         "Not superseded - the clipboard client it wraps is live in `os-sdk` and used there. This is the Tauri-plugin shell around it, waiting for an app to register the plugin."),
        ("sdk/i18n",
         "Built recently, consumer still to come."),
        ("daemons/integration-packages",
         "Built recently. IP-R5's manifest and permission-profile half; the installd side that would call it is not wired."),
        ("contracts/lenv",
         "Built recently. The .lenv parse and trust model, waiting on the transfer path that presents one."),
        ("contracts/file-change",
         "Built recently, consumer still to come."),
        ("daemons/sentinel-detect",
         "The pure detector core for an `org.arlen.Sentinel1` daemon that does not exist yet."),
        ("ai/ai-explanation",
         "System Explanation Mode, genuinely replaced rather than merely uncalled: `daemons/ai-engine-daemon/src/explain_iface.rs` serves `org.arlen.AI1.explain_system` by running the built-in explain skill on an ephemeral confined pi, and its own doc gives retiring this path as a reason it exists. The feature is alive; this implementation of it is the one awaiting removal."),
    ];

    /// Every `Cargo.toml` in the tree that declares a `[package]`.
    fn manifests(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                if n == "target" || n == "node_modules" || n.starts_with("mkosi") || n == ".git" {
                    continue;
                }
                manifests(&p, out);
            } else if p.file_name().is_some_and(|n| n == "Cargo.toml") {
                out.push(p);
            }
        }
    }

    /// A manifest with comments stripped, so a commented-out dependency does not
    /// read as a live one. `apps/settings/src-tauri` carries exactly that: a
    /// commented `arlen-config` line that would otherwise mask the finding.
    fn uncommented(text: &str) -> String {
        text.lines()
            .map(|l| l.split_once('#').map_or(l, |(before, _)| before))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A manifest without its `[package]` block, so the crate's own name does not
    /// read as a reference to a same-named crate elsewhere.
    fn strip_package_block(text: &str) -> String {
        let mut out = Vec::new();
        let mut in_package = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                in_package = l == "[package]";
            }
            if !in_package {
                out.push(line);
            }
        }
        out.join("\n")
    }

    /// The `name = "..."` of a manifest's `[package]`, if it has one.
    fn package_name(text: &str) -> Option<String> {
        let mut in_package = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                in_package = l == "[package]";
                continue;
            }
            if in_package {
                if let Some(rest) = l.strip_prefix("name") {
                    if let Some((_, v)) = rest.split_once('=') {
                        return Some(v.trim().trim_matches('"').to_string());
                    }
                }
            }
        }
        None
    }

    /// Whether `text` names `krate` as a whole token, so `arlen-config` does not
    /// match inside `arlen-config-format`.
    fn names(text: &str, krate: &str) -> bool {
        text.match_indices(krate).any(|(i, _)| {
            let before_ok = i == 0
                || !text[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
            let after = text[i + krate.len()..].chars().next();
            let after_ok = !after.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
            before_ok && after_ok
        })
    }

    /// A library crate with no binary and no dependent ships in nothing. It still
    /// compiles in CI and its tests still pass, so it reads as healthy from the
    /// outside; the only thing that distinguishes it from a live crate is that no
    /// manifest names it.
    ///
    /// Workspace membership does not count as consumption: `ai/Cargo.toml` lists
    /// `ai-explanation` as a member, which builds and tests it without anything
    /// calling it. Only a dependency edge counts, which is why the search skips
    /// the crate's own manifest and strips comments before looking.
    ///
    /// `dev/integration` is excluded because it is this test's own crate: a test
    /// harness is consumed by being run, not by being depended on.
    #[test]
    fn no_new_library_crate_is_unconsumed() {
        let root = repo_path("");
        let mut found = Vec::new();
        manifests(&root, &mut found);
        assert!(found.len() > 50, "expected the tree's manifests, got {}", found.len());

        let bodies: Vec<(PathBuf, String)> = found
            .iter()
            .filter_map(|m| std::fs::read_to_string(m).ok().map(|t| (m.clone(), uncommented(&t))))
            .collect();

        let mut unconsumed = Vec::new();
        for (manifest, text) in &bodies {
            let Some(name) = package_name(text) else {
                continue;
            };
            let dir = manifest.parent().expect("manifest has a directory");
            let rel = dir.strip_prefix(&root).unwrap_or(dir).to_string_lossy().to_string();
            if rel == "dev/integration" {
                continue;
            }
            let has_lib = dir.join("src/lib.rs").exists() || text.contains("[lib]");
            let has_bin = dir.join("src/main.rs").exists() || text.contains("[[bin]]");
            if !has_lib || has_bin {
                continue;
            }
            // A dependent is any OTHER manifest naming it outside its own
            // `[package]` block. Workspace member lists are not dependencies, so
            // the workspace root's mention does not count either.
            let depended = bodies.iter().any(|(other, other_text)| {
                if other == manifest {
                    return false;
                }
                // Another manifest's own `[package] name` is not a dependency on
                // this crate even when the two names are equal. Reading that as a
                // dependent is how this check first passed for a dead crate. The
                // pair that made it concrete - two crates both calling themselves
                // `arlen-system-monitor` - has since been renamed apart, but the
                // rule stands on its own: a name is not a dependency.
                let other_text = &strip_package_block(other_text);
                let without_members = other_text
                    .split("[workspace]")
                    .next()
                    .unwrap_or(other_text)
                    .to_string();
                let tail = other_text
                    .split_once("members")
                    .map_or(String::new(), |(_, t)| {
                        t.split_once(']').map_or(String::new(), |(_, r)| r.to_string())
                    });
                names(&without_members, &name) || names(&tail, &name)
            });
            if !depended {
                unconsumed.push(rel);
            }
        }
        unconsumed.sort();

        let known: Vec<String> = KNOWN_UNCONSUMED.iter().map(|(p, _)| p.to_string()).collect();
        let fresh: Vec<&String> = unconsumed.iter().filter(|u| !known.contains(u)).collect();
        assert!(
            fresh.is_empty(),
            "library crates that build and test but nothing depends on ({}):\n  {}\n\
             Give it a consumer, or add it to KNOWN_UNCONSUMED with the reason it waits.",
            fresh.len(),
            fresh.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  ")
        );

        let stale: Vec<&String> = known.iter().filter(|k| !unconsumed.contains(k)).collect();
        assert!(
            stale.is_empty(),
            "KNOWN_UNCONSUMED lists crates that now have a dependent ({}):\n  {}\n\
             Delete these entries: the list is meant to shrink.",
            stale.len(),
            stale.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  ")
        );
    }
}

#[cfg(test)]
mod terminal_read_scope_agreement {
    use arlen_terminal_core::read_serve::ReadRequest;

    /// The read half has three parties that must agree on what a scope is: the
    /// daemon digests one to mint a token, the MCP tool forwards a request, and
    /// the terminal digests the request to verify. They are three separate
    /// parsers in three crates, and if any of them disagreed about a default the
    /// digests would differ and every read would fail as an unexplained refusal,
    /// with each side believing it was right.
    ///
    /// The dangerous direction is a default drifting OPEN: if one side treated an
    /// absent `include_user_blocks` as true, a token would authorize a reading
    /// wider than the one performed. So this pins the shape rather than trusting
    /// three `#[serde(default)]` attributes to stay in step.
    #[test]
    fn an_omitted_flag_means_the_same_narrow_thing_to_every_party() {
        // What the MCP tool sends when the model asks for the minimal reading.
        let wire = r#"{"terminal_id":"t1","limit":3,"consent":"tok"}"#;
        let req: ReadRequest = serde_json::from_str(wire).expect("the wire shape parses");
        assert!(!req.include_user_blocks, "an omitted flag must not widen the reading");
        assert!(!req.include_running);

        // The terminal digests the request; the daemon digests what it minted for.
        // Same values, same digest - that equality is the contract.
        let from_request = arlen_run_consent_token::read_digest(
            &req.terminal_id,
            u32::try_from(req.limit).expect("a test limit fits"),
            req.include_user_blocks,
            req.include_running,
        );
        let from_mint = arlen_run_consent_token::read_digest("t1", 3, false, false);
        assert_eq!(from_request, from_mint);
    }

    /// And a token minted for the narrow reading must not verify against a wider
    /// request, which is the same property one layer up: the digests differ, so
    /// the biscuit refuses.
    #[test]
    fn a_narrow_token_does_not_verify_a_widened_request() {
        let narrow = arlen_run_consent_token::read_digest("t1", 3, false, false);
        for widened in [
            arlen_run_consent_token::read_digest("t1", 3, true, false),
            arlen_run_consent_token::read_digest("t1", 3, false, true),
            arlen_run_consent_token::read_digest("t1", 4, false, false),
            arlen_run_consent_token::read_digest("t2", 3, false, false),
        ] {
            assert_ne!(narrow, widened);
        }
    }
}
