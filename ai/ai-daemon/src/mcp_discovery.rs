//! MCP server discovery for the AI daemon.
//!
//! Tier-1 `mcp.server` modules are hosted by `arlen-modulesd`,
//! which fronts each with a Unix socket under
//! `$XDG_RUNTIME_DIR/arlen/mcp/modules/` and announces it on the
//! Event Bus. This module keeps the daemon's [`McpClient`] in step
//! with that feed.
//!
//! Trust model: the Event Bus does not authenticate event *content*
//! (any same-uid producer can emit `module.installed`), and the
//! modules directory is writable by any same-uid process. Discovery
//! therefore trusts neither surface blindly. Before registering a
//! server it (1) rejects module ids that could escape the socket
//! directory and (2) resolves the socket's server peer via
//! `SO_PEERCRED` and requires it to be `modulesd`. A forged event or
//! a stray socket cannot make the daemon adopt an imposter server.
//!
//! Per foundation §5.7 a third-party module MCP server is always
//! treated as an action server until it carries a Security Audit
//! Badge, so every module is registered with [`ServerClass::Action`].

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use arlen_ai_core::audit::AuditSink;
use arlen_ai_core::mcp::{McpClient, ServerClass, ServerId};
use arlen_permissions::identity::app_id_from_pid;
use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};
use os_sdk::mcp::{is_safe_module_id, mcp_module_socket_path, mcp_socket_path};
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Event-type prefix the discovery loop subscribes to. Prefix-match
/// semantics: this catches `module.installed`, `module.removed`, and
/// any future `module.*` event.
const MODULE_NAMESPACE: &str = "module.";

/// Resolved `app_id` of the canonically-installed module runtime
/// daemon. `arlen-permissions` maps `/usr/bin/arlen-modulesd` to
/// this; a module MCP socket served by anything else is an imposter.
const MODULESD_APP_ID: &str = "modulesd";

/// Wait budget for opening and handshaking a module socket. A bad
/// socket that accepts but stalls cannot wedge discovery.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Backoff between Event Bus subscribe attempts. The bus may come up
/// after the daemon during a normal boot; discovery keeps retrying
/// rather than disabling itself permanently.
const SUBSCRIBE_RETRY: Duration = Duration::from_secs(5);

/// Well-known system MCP servers connected at startup. Unlike Tier-1
/// module servers (discovered dynamically over the Event Bus), the
/// Arlen-shipped servers live at fixed socket ids and are read-only
/// (`mcp-server-layer.md` §2, §4.1 default-permit). Each entry is the
/// socket id and the `app_id` the server's process must resolve to.
const SYSTEM_SERVERS: &[(&str, &str)] = &[
    ("system.knowledge", KNOWLEDGE_MCP_APP_ID),
    ("system.monitor", SYSTEM_MONITOR_MCP_APP_ID),
];

/// Resolved `app_id` of the canonically-installed Knowledge Graph MCP
/// server. `arlen-permissions` maps `/usr/bin/arlen-knowledge-mcp` to
/// this; a `system.knowledge` socket served by anything else is an
/// imposter and is refused.
const KNOWLEDGE_MCP_APP_ID: &str = "knowledge-mcp";

/// Resolved `app_id` of the canonically-installed System Monitor MCP
/// server (`/usr/bin/arlen-system-monitor-mcp`). It is read-only and
/// exposes only system-wide public info (process list, load, memory,
/// disk), so unlike the File Manager it carries no per-query scope to
/// bypass and is safe to register as a default-permit `ReadOnly` server.
/// A `system.monitor` socket served by anything else is an imposter.
const SYSTEM_MONITOR_MCP_APP_ID: &str = "system-monitor-mcp";

/// Backoff between attempts to reach a system server that has not come
/// up yet. System daemons can start after the AI daemon during boot.
const SYSTEM_CONNECT_RETRY: Duration = Duration::from_secs(5);

/// Interval between liveness checks on a connected system server. A
/// server that is restarted (update, crash recovery) leaves a dead
/// transport behind; the supervisor notices on the next check and
/// reconnects, so the AI daemon never needs a restart to recover.
const SYSTEM_HEALTH_INTERVAL: Duration = Duration::from_secs(30);

/// Keeps the daemon's [`McpClient`] connected to the set of
/// currently-hosted Tier-1 module MCP servers.
pub struct McpDiscovery {
    client: Arc<Mutex<McpClient>>,
}

impl McpDiscovery {
    /// Build a discovery handle around a fresh, empty client.
    ///
    /// `audit` is wired into that client so every tool call dispatched
    /// through a discovered module server commits a content-free
    /// audit-ledger entry.
    pub fn new(audit: Arc<dyn AuditSink>) -> Self {
        Self {
            client: Arc::new(Mutex::new(McpClient::new().with_audit(audit))),
        }
    }

    /// The shared MCP client. The query path dispatches tool calls
    /// through this once AI-side tool routing lands; until then the
    /// discovery loop is its only writer.
    pub fn client(&self) -> Arc<Mutex<McpClient>> {
        Arc::clone(&self.client)
    }

    /// Subscribe to the `module.` Event Bus namespace and keep the
    /// client in step. Subscription is retried until it succeeds, and
    /// re-established if the feed later closes; an existing-socket
    /// reconciliation runs on every (re)subscribe. Runs forever.
    pub async fn run(self: Arc<Self>, consumer: UnixEventConsumer) {
        // Connect the well-known system servers concurrently with module
        // discovery. They retry independently until reachable, so a system
        // daemon that starts after this one is still picked up.
        for &(server_id, expected_app_id) in SYSTEM_SERVERS {
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                this.connect_system_server_with_retry(server_id, expected_app_id)
                    .await;
            });
        }
        loop {
            let mut rx = match consumer
                .subscribe(vec![MODULE_NAMESPACE.to_string()])
                .await
            {
                Ok(rx) => rx,
                Err(err) => {
                    warn!(
                        "mcp discovery: event bus subscribe failed: {err}; \
                         retrying in {}s",
                        SUBSCRIBE_RETRY.as_secs()
                    );
                    tokio::time::sleep(SUBSCRIBE_RETRY).await;
                    continue;
                }
            };
            info!("mcp discovery: subscribed to the module.* event namespace");
            // Reconcile *after* subscribing: an install or remove that
            // lands during the scan queues in `rx` and is drained by
            // the loop below, so the startup gap drops no event.
            self.scan_existing().await;
            while let Some(event) = rx.recv().await {
                self.handle_event(&event.r#type, &event.payload).await;
            }
            warn!("mcp discovery: event feed closed; re-subscribing");
        }
    }

    /// Dispatch one `module.*` event.
    async fn handle_event(&self, event_type: &str, payload: &[u8]) {
        // modulesd carries the module id as the raw UTF-8 payload.
        let module_id = match std::str::from_utf8(payload) {
            Ok(id) if !id.is_empty() => id,
            _ => {
                warn!(
                    event_type,
                    "mcp discovery: module event with no valid module id"
                );
                return;
            }
        };
        match event_type {
            "module.installed" => self.connect_module(module_id).await,
            "module.removed" => self.disconnect_module(module_id).await,
            other => debug!(
                event_type = other,
                "mcp discovery: non-discovery module event ignored"
            ),
        }
    }

    /// Connect to every module MCP socket already present on disk.
    /// Covers modules hosted before the daemon started, and re-runs
    /// on every resubscribe as a reconciliation pass.
    async fn scan_existing(&self) {
        let Some(dir) = mcp_module_socket_path("placeholder")
            .parent()
            .map(|p| p.to_path_buf())
        else {
            return;
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // A missing directory just means no module has been
            // hosted yet; that is not an error.
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sock") {
                continue;
            }
            if let Some(module_id) = path.file_stem().and_then(|s| s.to_str()) {
                self.connect_module(module_id).await;
            }
        }
    }

    /// Connect the client to one module's MCP socket, after checking
    /// the id is path-safe and the socket is actually served by
    /// modulesd. Any failure is logged, not fatal.
    async fn connect_module(&self, module_id: &str) {
        // (1) An id that fails this check could format a socket path
        // outside the modules directory. modulesd applies the same
        // gate before binding; discovery does not trust an id it has
        // not validated itself.
        if !is_safe_module_id(module_id) {
            warn!(module = module_id, "mcp discovery: unsafe module id, ignored");
            return;
        }
        let path = mcp_module_socket_path(module_id);

        // Open the stream directly so the server peer can be checked
        // before the rmcp handshake runs.
        let stream = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            UnixStream::connect(&path),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                warn!(
                    module = module_id,
                    "mcp discovery: module server unavailable: {err}"
                );
                return;
            }
            Err(_elapsed) => {
                warn!(module = module_id, "mcp discovery: connect timed out");
                return;
            }
        };

        // (2) Authenticate the server end. A stray socket planted by
        // another same-uid process is served by that process, not
        // modulesd, and is refused here.
        if !peer_is_modulesd(&stream) {
            warn!(
                module = module_id,
                "mcp discovery: socket is not served by modulesd; refusing"
            );
            return;
        }

        let mut client = self.client.lock().await;
        match tokio::time::timeout(
            CONNECT_TIMEOUT,
            client.connect_stream(
                ServerId(module_id.to_string()),
                stream,
                ServerClass::Action,
            ),
        )
        .await
        {
            Ok(Ok(())) => info!(module = module_id, "mcp discovery: connected"),
            Ok(Err(err)) => warn!(
                module = module_id,
                "mcp discovery: handshake failed: {err}"
            ),
            Err(_elapsed) => warn!(
                module = module_id,
                "mcp discovery: handshake timed out"
            ),
        }
    }

    /// Supervise one well-known system server for the daemon's lifetime.
    ///
    /// Connects (retrying while the server is not yet up, so a daemon that
    /// starts after this one is still picked up), then health-checks the
    /// live connection on an interval. If the server is restarted the
    /// transport goes dead; the check notices, drops the stale connection,
    /// and the loop reconnects, so the AI daemon recovers without a restart.
    /// Runs forever.
    async fn connect_system_server_with_retry(
        self: Arc<Self>,
        server_id: &str,
        expected_app_id: &str,
    ) {
        let id = ServerId(server_id.to_string());
        let path = mcp_socket_path(server_id);
        loop {
            // Only a read-only registration counts as *our* system server.
            // If some other connection occupied this id (it should not, system
            // ids are reserved from module discovery), it is not the system
            // server, so reconnect to restore the authenticated read-only one.
            let connected = matches!(
                self.client.lock().await.server_class(&id),
                Some(ServerClass::ReadOnly)
            );
            if !connected {
                self.connect_system_server(server_id, expected_app_id, &path)
                    .await;
                tokio::time::sleep(SYSTEM_CONNECT_RETRY).await;
                continue;
            }
            // Connected: wait, then verify the transport is still alive with
            // a cheap tools listing. A failure means the server went away.
            // Bounded by a timeout so a hung transport cannot hold the client
            // lock indefinitely.
            tokio::time::sleep(SYSTEM_HEALTH_INTERVAL).await;
            let healthy = {
                let client = self.client.lock().await;
                matches!(
                    tokio::time::timeout(CONNECT_TIMEOUT, client.list_tools(&id)).await,
                    Ok(Ok(_))
                )
            };
            if !healthy {
                self.client.lock().await.disconnect(&id);
                warn!(
                    server = server_id,
                    "mcp discovery: system server connection lost; reconnecting"
                );
            }
        }
    }

    /// Connect the client to one well-known system server, after
    /// verifying the socket is served by the expected daemon. System
    /// servers are read-only (default-permit). Returns whether the
    /// connection was established. Any failure is logged, not fatal.
    async fn connect_system_server(
        &self,
        server_id: &str,
        expected_app_id: &str,
        path: &Path,
    ) -> bool {
        // Open the stream directly so the server peer can be checked
        // before the rmcp handshake runs.
        let stream = match tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(path)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) => return false, // not up yet; the caller retries
            Err(_elapsed) => {
                warn!(server = server_id, "mcp discovery: system connect timed out");
                return false;
            }
        };

        // Authenticate the server end. A socket planted by another
        // same-uid process is served by that process, not the system
        // daemon, and is refused.
        if !peer_is_app(&stream, expected_app_id) {
            warn!(
                server = server_id,
                "mcp discovery: socket is not served by {expected_app_id}; refusing"
            );
            return false;
        }

        let mut client = self.client.lock().await;
        match tokio::time::timeout(
            CONNECT_TIMEOUT,
            client.connect_stream(
                ServerId(server_id.to_string()),
                stream,
                ServerClass::ReadOnly,
            ),
        )
        .await
        {
            Ok(Ok(())) => {
                info!(server = server_id, "mcp discovery: system server connected");
                true
            }
            Ok(Err(err)) => {
                warn!(server = server_id, "mcp discovery: handshake failed: {err}");
                false
            }
            Err(_elapsed) => {
                warn!(server = server_id, "mcp discovery: handshake timed out");
                false
            }
        }
    }

    /// Drop the client's connection to one module.
    async fn disconnect_module(&self, module_id: &str) {
        self.client
            .lock()
            .await
            .disconnect(&ServerId(module_id.to_string()));
        info!(module = module_id, "mcp discovery: disconnected");
    }
}

/// Whether the peer that bound `stream`'s server end is `modulesd`.
///
/// Uses `SO_PEERCRED` on the live connection — the credentials of the
/// process actually `accept()`ing — so it cannot be spoofed by a
/// path swap. In debug builds every component runs from a cargo
/// target directory and resolves to a `dev.*` id, so those pass too.
fn peer_is_modulesd(stream: &UnixStream) -> bool {
    peer_is_app(stream, MODULESD_APP_ID)
}

/// Whether the peer that bound `stream`'s server end resolves to
/// `expected_app_id`.
///
/// Uses `SO_PEERCRED` on the live connection so it cannot be spoofed by
/// a path swap. In debug builds every component runs from a cargo
/// target directory and resolves to a `dev.*` id, so those pass too.
fn peer_is_app(stream: &UnixStream, expected_app_id: &str) -> bool {
    let Ok(cred) = stream.peer_cred() else {
        return false;
    };
    let Some(pid) = cred.pid() else {
        return false;
    };
    if pid < 0 {
        return false;
    }
    let Ok(app_id) = app_id_from_pid(pid as u32) else {
        return false;
    };
    app_id == expected_app_id
        || (cfg!(debug_assertions) && app_id.starts_with("dev."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_proto::MockAuditSink;
    use os_sdk::mcp::rmcp;
    use os_sdk::mcp::serve_mcp_at;
    use rmcp::ServerHandler;
    use std::path::PathBuf;

    /// Minimal MCP server: the default handler is enough to complete the
    /// initialize handshake, which is all the connect path needs.
    #[derive(Clone)]
    struct TestServer;
    impl ServerHandler for TestServer {}

    fn discovery() -> Arc<McpDiscovery> {
        Arc::new(McpDiscovery::new(Arc::new(MockAuditSink::accepting())))
    }

    fn temp_socket(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("arlen-sysmcp-{tag}-{}-{unique}", std::process::id()))
            .join("s.sock")
    }

    #[test]
    fn system_servers_include_the_monitor_with_its_canonical_app_id() {
        // The monitor is wired for discovery, and its expected peer app id is
        // the one its `/usr/bin/arlen-system-monitor-mcp` install path resolves
        // to (the imposter check rejects any other server on that socket).
        assert!(SYSTEM_SERVERS.contains(&("system.monitor", SYSTEM_MONITOR_MCP_APP_ID)));
        assert_eq!(SYSTEM_MONITOR_MCP_APP_ID, "system-monitor-mcp");
    }

    #[tokio::test]
    async fn connect_system_server_refuses_an_absent_socket() {
        // Nothing is listening: the connect attempt fails cleanly and the
        // supervisor retries rather than registering a dead connection.
        let disc = discovery();
        let ok = disc
            .connect_system_server("system.knowledge", KNOWLEDGE_MCP_APP_ID, &temp_socket("absent"))
            .await;
        assert!(!ok);
        assert!(disc.client().lock().await.is_empty());
    }

    #[tokio::test]
    async fn connect_system_server_connects_and_registers_read_only() {
        let socket = temp_socket("live");
        let socket_for_task = socket.clone();
        let server = tokio::spawn(async move {
            let _ = serve_mcp_at(&socket_for_task, || TestServer).await;
        });
        // Wait for the server to bind.
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(socket.exists(), "test server did not bind");

        // The server runs in this same test process, so its peer identity
        // resolves to a `dev.*` id, which the connect path admits in debug.
        let disc = discovery();
        let ok = disc
            .connect_system_server("system.knowledge", KNOWLEDGE_MCP_APP_ID, &socket)
            .await;
        assert!(ok, "should connect to a live peer-authed server");

        let id = ServerId("system.knowledge".to_string());
        assert_eq!(
            disc.client().lock().await.server_class(&id),
            Some(ServerClass::ReadOnly),
            "system server must register as read-only (default-permit)"
        );

        server.abort();
    }
}
