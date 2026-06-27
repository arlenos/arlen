//! `arlen-bridge-ingest`: the foreign-app bridge ingestion daemon
//! (foreign-app-bridges piece 2). One instance per installed bridge: it loads
//! the bridge's declarative `bridge.toml`, speaks the native-messaging stdio
//! protocol to the foreign plugin (mutual-id-pin handshake + untrusted-message
//! validation), interprets each inbound message against the mapping, and
//! persists the resulting upserts into the Knowledge Graph through the app-tier
//! entity-write socket.
//!
//! The privileged side runs no per-bridge code: a bridge is data. Every write
//! is namespace-bound + origin-tagged daemon-side by this process's attested
//! caller identity (a bridge can only write its own declared namespace, never a
//! `system.*` fact). Edges are ingested too: a mapping's `for_each_link` rule
//! becomes an idempotent edge through the `plan_entity_link` knowledge op (the
//! `link_entities` client call). A write that fails reports the message failed
//! (the session continues) rather than silently dropping the node or edge.

use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;
use std::process::ExitCode;

use arlen_bridge_ingest::sink::EntityWriter;
use arlen_bridge_ingest::{BridgeConfig, KgPlanSink};
use os_sdk::UnixGraphClient;
use serde_json::{Map, Value};

/// Resolve the knowledge daemon's write socket: `ARLEN_DAEMON_SOCKET`, else
/// `$XDG_RUNTIME_DIR/arlen/knowledge.sock`, else `/run/arlen/knowledge.sock`.
fn knowledge_socket() -> String {
    if let Some(s) = std::env::var_os("ARLEN_DAEMON_SOCKET") {
        return s.to_string_lossy().into_owned();
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let mut p = PathBuf::from(dir);
        p.push("arlen/knowledge.sock");
        return p.to_string_lossy().into_owned();
    }
    "/run/arlen/knowledge.sock".to_string()
}

/// An [`EntityWriter`] that persists through the knowledge daemon's app-tier
/// entity-write socket. The host loop is synchronous, so this owns a
/// current-thread runtime and blocks on the async client (no nested runtime,
/// so `block_on` is safe). Both `upsert` (`upsert_entity`) and `link`
/// (`link_entities`, the `plan_entity_link` op) drive the async client.
struct GraphEntityWriter {
    client: UnixGraphClient,
    runtime: tokio::runtime::Runtime,
}

impl GraphEntityWriter {
    fn new(socket_path: String) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(Self {
            client: UnixGraphClient::new(socket_path),
            runtime,
        })
    }
}

impl EntityWriter for GraphEntityWriter {
    fn upsert(
        &mut self,
        qualified_type: &str,
        external_key: &str,
        fields: &Map<String, Value>,
    ) -> Result<(), String> {
        self.runtime
            .block_on(self.client.upsert_entity(qualified_type, external_key, fields))
            .map_err(|e| e.to_string())
    }

    fn link(
        &mut self,
        edge: &str,
        from_type: &str,
        from_key: &str,
        to_type: &str,
        to_key: &str,
    ) -> Result<(), String> {
        self.runtime
            .block_on(
                self.client
                    .link_entities(edge, from_type, from_key, to_type, to_key),
            )
            .map_err(|e| e.to_string())
    }
}

fn run() -> Result<(), String> {
    // The bridge.toml path: the first CLI argument, else $ARLEN_BRIDGE_CONFIG.
    let config_path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("ARLEN_BRIDGE_CONFIG").ok())
        .ok_or_else(|| {
            "usage: arlen-bridge-ingest <bridge.toml> (or set ARLEN_BRIDGE_CONFIG)".to_string()
        })?;
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("reading {config_path}: {e}"))?;
    let config = BridgeConfig::parse(&text).map_err(|e| format!("bridge config: {e}"))?;

    let writer = GraphEntityWriter::new(knowledge_socket())
        .map_err(|e| format!("graph client runtime: {e}"))?;
    let mut sink = KgPlanSink::new(writer);

    tracing::info!(
        plugin = %config.bridge.allowed_plugin_id,
        "bridge ingest host ready"
    );

    // Two drivers reach the same interpret -> sink path. If a vault is configured
    // (`ARLEN_OBSIDIAN_VAULT`), run the daemon-side FLOOR: an initial sync plus a
    // live `.md` file-watch (foreign-app-bridges.md). Otherwise run the generic
    // plugin transport: native-messaging frames over stdin, replies over stdout.
    if let Some(vault) = std::env::var_os("ARLEN_OBSIDIAN_VAULT") {
        let vault = PathBuf::from(vault);
        tracing::info!(vault = %vault.display(), "obsidian floor: watching vault");
        return arlen_bridge_ingest::watch_vault(&vault, &config, &mut sink)
            .map_err(|e| format!("vault watch: {e}"));
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    arlen_bridge_ingest::serve(&config, &mut reader, &mut writer, &mut sink)
        .map_err(|e| format!("host loop: {e}"))
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_write_to_a_dead_socket_surfaces_a_transport_error() {
        // link drives the async link_entities client through block_on; a dead
        // socket surfaces the transport error as a string (a real link is covered
        // by the os-sdk link_entities fake-daemon test).
        let mut w = GraphEntityWriter::new("/nonexistent-bridge-link.sock".to_string()).unwrap();
        let err = w
            .link("LINKS_TO", "md.obsidian.Note", "note-1", "md.obsidian.Note", "note-2")
            .expect_err("a dead socket cannot link");
        assert!(!err.is_empty(), "the transport error is surfaced as a string");
    }

    #[test]
    fn an_upsert_to_a_dead_socket_surfaces_a_transport_error() {
        // The block_on bridge from the sync host to the async client works (the
        // error path here proves the round trip is driven); a real upsert is
        // covered by the os-sdk client's own fake-daemon test.
        let mut w = GraphEntityWriter::new("/nonexistent-bridge-sock.sock".to_string()).unwrap();
        let fields = Map::new();
        let err = w
            .upsert("md.obsidian.Note", "note-1", &fields)
            .expect_err("a dead socket cannot upsert");
        assert!(!err.is_empty(), "the transport error is surfaced as a string");
    }
}
