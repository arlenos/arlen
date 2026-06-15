//! `arlen-code-indexer` - the Tier-2 code-graph ingestion daemon
//! (code-graph-layer.md CG-R1).
//!
//! Consumes `file.opened` from the event bus; for each project-scoped source
//! file (Rust, Python, TypeScript) whose content has changed since it was last
//! seen, it re-parses ONLY that file (per-file isolation) and emits a
//! `code.indexed` event the knowledge daemon promotes into `CodeSymbol` +
//! `DEFINES`. Per-user daemon (the journald-parser shape): it reads the user's
//! own project source and emits onto the per-uid event bus.
//!
//! The anti-Nepomuk guardrails (§6) live here: only supported source extensions
//! (`language_for_path`), only files under a live `Project` root (never the
//! whole disk), skipping build/cache/VCS dirs, and an mtime de-dup so a mere
//! re-open of an unchanged file does not re-parse. The project roots are cached
//! from the graph and refreshed on a slow timer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use arlen_code_indexer::extract::extract;
use arlen_code_indexer::index::{
    build_payload, language_for_path, path_has_traversal, path_in_ignored_dir, path_under_any,
    was_truncated, MAX_FILE_BYTES,
};
use os_sdk::event::{EventEmitter, UnixEventEmitter};
use os_sdk::graph::UnixGraphClient;
use os_sdk::proto::{Event, FileOpenedPayload};
use prost::Message as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::{debug, info, warn};

/// How long a cached set of project roots is trusted before a refresh. Projects
/// change rarely; a coarse refresh keeps the scope current without a per-event
/// graph query.
const PROJECT_ROOTS_TTL: Duration = Duration::from_secs(60);

/// The event-bus frame cap (matches the writer / bus `MAX_FRAME`).
const MAX_FRAME: usize = 1024 * 1024;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let producer = os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");
    let consumer = os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock");
    let graph_socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    info!(consumer = %consumer.display(), "code-indexer starting");

    let emitter = UnixEventEmitter::new(producer.to_string_lossy().into_owned());
    let graph = UnixGraphClient::new(graph_socket.to_string_lossy().into_owned());

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    let consumer_socket = consumer.to_string_lossy().into_owned();
    let mut indexer = Indexer::new(emitter, graph);
    tokio::select! {
        _ = indexer.run(&consumer_socket) => {}
        _ = shutdown_signal() => info!("code-indexer shutting down"),
    }
}

/// The running indexer: the bus emitter, the graph client (for project scope),
/// the cached project roots and the per-file last-indexed mtimes.
struct Indexer {
    emitter: UnixEventEmitter,
    graph: UnixGraphClient,
    roots: Vec<String>,
    roots_refreshed: Option<SystemTime>,
    last_indexed: HashMap<PathBuf, SystemTime>,
}

impl Indexer {
    fn new(emitter: UnixEventEmitter, graph: UnixGraphClient) -> Self {
        Self {
            emitter,
            graph,
            roots: Vec::new(),
            roots_refreshed: None,
            last_indexed: HashMap::new(),
        }
    }

    /// Connect, register and consume `file.opened` forever, reconnecting on a
    /// bus drop. Never returns in normal operation.
    async fn run(&mut self, consumer_socket: &str) {
        loop {
            if let Err(e) = self.consume(consumer_socket).await {
                warn!("consumer error: {e}; reconnecting in 2s");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// One connection's lifetime: register as a `file.opened` consumer and pump
    /// events until the stream closes or errors.
    async fn consume(&mut self, consumer_socket: &str) -> std::io::Result<()> {
        let mut stream = UnixStream::connect(consumer_socket).await?;
        // The bus reads three newline lines: consumer id, patterns, UID filter.
        stream.write_all(b"code-indexer\n").await?;
        stream.write_all(b"file.opened\n").await?;
        stream.write_all(b"*\n").await?;
        info!("registered as a file.opened consumer");

        loop {
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e),
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            if len == 0 || len > MAX_FRAME {
                return Err(std::io::Error::other("invalid event frame length"));
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            if let Ok(event) = Event::decode(buf.as_slice()) {
                self.handle_event(&event).await;
            }
        }
    }

    /// Index one `file.opened` event if it is a changed, project-scoped Rust file.
    async fn handle_event(&mut self, event: &Event) {
        if event.r#type != "file.opened" {
            return;
        }
        let Ok(payload) = FileOpenedPayload::decode(event.payload.as_slice()) else {
            return;
        };
        let path = payload.path;
        if path.is_empty() {
            return;
        }
        // Only index a supported source language (Rust / Python / TypeScript);
        // anything else is skipped without spawning a parse.
        let Some(language) = language_for_path(&path) else {
            return;
        };
        // Reject a `..`-traversal path: the project-scope check below is textual,
        // so a traversal that is textually under a root must be dropped here lest
        // it read a file outside the project (the per-file isolation / §6 scope).
        if path_has_traversal(&path) {
            return;
        }
        // Skip build outputs / dependency caches / VCS dirs even under a project
        // root: a generated `target/**/out.rs` or a vendored dep is not authored
        // code, and parsing it on every build is the unbounded-cost trap
        // (prior-art-lessons.md §3 guardrail 1).
        if path_in_ignored_dir(&path) {
            return;
        }

        // Project scope: only index files under a live project root.
        self.refresh_roots_if_stale().await;
        if !path_under_any(&path, &self.roots) {
            return;
        }

        // mtime de-dup: skip a re-open of a file we already indexed unchanged.
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(_) => return, // gone / unreadable: nothing to index
        };
        // Cost budget (§6): never read a giant / generated file whole into RAM.
        if meta.len() > MAX_FILE_BYTES {
            debug!(%path, bytes = meta.len(), "skipping oversized file");
            return;
        }
        let mtime = meta.modified().ok();
        let key = PathBuf::from(&path);
        if let (Some(mt), Some(prev)) = (mtime, self.last_indexed.get(&key)) {
            if mt == *prev {
                return; // unchanged since last index
            }
        }

        // Read + parse ONLY this file, in isolation, and emit the index.
        let source = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) => {
                debug!(%path, "read failed: {e}");
                return;
            }
        };
        let file_index = extract(language, &source);
        if was_truncated(&file_index) {
            warn!(%path, symbols = file_index.symbols.len(), "file exceeds the symbol cap; indexing a truncated set");
        }
        let payload = build_payload(&path, language, &file_index);
        let symbols = payload.symbols.len();
        match self.emitter.emit("code.indexed", payload.encode_to_vec()).await {
            Ok(()) => {
                if let Some(mt) = mtime {
                    self.last_indexed.insert(key, mt);
                }
                debug!(%path, symbols, "emitted code.indexed");
            }
            Err(e) => warn!(%path, "code.indexed emit failed: {e}"),
        }
    }

    /// Refresh the cached project roots from the graph if the TTL has elapsed.
    /// Best-effort: an unavailable graph keeps the prior roots (scope degrades
    /// gracefully, never indexes the whole disk).
    async fn refresh_roots_if_stale(&mut self) {
        let stale = match self.roots_refreshed {
            None => true,
            Some(t) => t.elapsed().map(|e| e >= PROJECT_ROOTS_TTL).unwrap_or(true),
        };
        if !stale {
            return;
        }
        let cypher = "MATCH (p:Project) WHERE p.expired_at IS NULL \
                      AND p.root_path IS NOT NULL RETURN p.root_path AS root";
        match self.graph.query_rows(cypher).await {
            Ok(rows) => {
                self.roots = rows
                    .iter()
                    .filter_map(|r| r.get("root").and_then(|v| v.as_str()).map(str::to_string))
                    .filter(|s| !s.is_empty())
                    .collect();
                self.roots_refreshed = Some(SystemTime::now());
                debug!(count = self.roots.len(), "refreshed project roots");
            }
            Err(e) => debug!("project-root refresh skipped: {e}"),
        }
    }
}

/// Resolve when the process receives SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
