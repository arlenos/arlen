//! The timeline FUSE helper.
//!
//! A small, UNFENCED process that serves the `~/.timeline` view by querying the
//! knowledge daemon's read socket. It is split out of the main knowledge daemon
//! (same-uid-isolation-plan.md option b) so that daemon can take the Landlock +
//! `no_new_privs` write-fence over its real write-set (`events.db`, the ladybug
//! graph dir, the project store, the FTS index). That fence's mandatory
//! `no_new_privs` neuters the SUID `fusermount3` the FUSE mount needs, so the
//! mount lives here, unfenced, where the helper holds no write capability of its
//! own - it only reads the graph over the daemon's read socket.
//!
//! The timeline is optional: an empty or `off` `ARLEN_TIMELINE_MOUNT` disables it
//! (the helper exits cleanly), matching the daemon's prior behaviour.

use anyhow::{anyhow, Context, Result};
use knowledge::fuse::{self, SyncGraphReader};
use knowledge::graph::{CellValue, RowSet};
use os_sdk::graph::UnixGraphClient;
use tracing::{info, warn};

const DEFAULT_TIMELINE_MOUNT: &str = ".timeline";

/// A [`SyncGraphReader`] backed by the knowledge daemon's read socket. The FUSE
/// callbacks are synchronous, so each query blocks on a small current-thread
/// tokio runtime that drives the async [`UnixGraphClient`]. Read-only by
/// construction: the client speaks only the daemon's typed read mode.
struct SocketGraphReader {
    rt: tokio::runtime::Runtime,
    client: UnixGraphClient,
}

impl SocketGraphReader {
    fn connect(socket_path: String) -> Result<Self> {
        // current_thread: the FUSE callbacks are synchronous and serialised, so a
        // single-threaded runtime driven by block_on is all the helper needs.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build timeline tokio runtime")?;
        let client = UnixGraphClient::new(socket_path);
        Ok(SocketGraphReader { rt, client })
    }
}

impl SyncGraphReader for SocketGraphReader {
    fn query_rows_sync(&self, cypher: String) -> Result<RowSet> {
        let (columns, rows) = self
            .rt
            .block_on(self.client.query_rows_ordered(&cypher))
            .map_err(|e| anyhow!("timeline graph read failed: {e}"))?;
        let rows = rows
            .into_iter()
            .map(|cells| cells.into_iter().map(json_to_cell).collect())
            .collect();
        Ok(RowSet { columns, rows })
    }
}

/// Map a typed JSON cell from the daemon's read response to the knowledge
/// daemon's own [`CellValue`]. The daemon's typed mode emits only scalars (it
/// fails closed on complex/temporal values), so an array or object never arises
/// for real node fields; map it to `Null` fail-safe rather than panicking.
fn json_to_cell(v: serde_json::Value) -> CellValue {
    use serde_json::Value as J;
    match v {
        J::Null => CellValue::Null,
        J::String(s) => CellValue::String(s),
        J::Bool(b) => CellValue::Bool(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                CellValue::Int64(i)
            } else if let Some(u) = n.as_u64() {
                CellValue::Int64(u as i64)
            } else if let Some(f) = n.as_f64() {
                CellValue::Float(f)
            } else {
                CellValue::Null
            }
        }
        J::Array(_) | J::Object(_) => CellValue::Null,
    }
}

/// Resolve the timeline mount path the same way the daemon did: an empty or
/// `off` `ARLEN_TIMELINE_MOUNT` disables it; an explicit value is used verbatim;
/// unset falls back to `$HOME/.timeline`.
fn timeline_mount() -> Option<String> {
    match std::env::var("ARLEN_TIMELINE_MOUNT") {
        Ok(v) if v.is_empty() || v == "off" => None,
        Ok(v) => Some(v),
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            Some(format!("{home}/{DEFAULT_TIMELINE_MOUNT}"))
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Some(mount_path) = timeline_mount() else {
        info!("timeline FUSE mount disabled (ARLEN_TIMELINE_MOUNT empty/off); helper exiting");
        return Ok(());
    };

    // The daemon's read socket: ARLEN_DAEMON_SOCKET wins, else the per-user
    // $XDG_RUNTIME_DIR/arlen/knowledge.sock, else /run/arlen/knowledge.sock -
    // the exact resolution the daemon binds with.
    let socket = knowledge::utils::socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock");
    info!(%socket, %mount_path, "timeline helper: connecting to the knowledge read socket");

    let reader = SocketGraphReader::connect(socket)?;
    // Blocks for the lifetime of the mount; returns when the filesystem is
    // unmounted. A mount failure (e.g. a stale mount) is surfaced as an error.
    if let Err(e) = fuse::mount(&mount_path, reader) {
        warn!("timeline FUSE mount failed: {e}");
        return Err(e);
    }
    Ok(())
}
