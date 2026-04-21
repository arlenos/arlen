/// Recent-files list sourced from the Knowledge Graph.
///
/// Lunaris tracks every `file.opened` event system-wide via eBPF →
/// Event Bus → Knowledge daemon (`file.opened` events are promoted to
/// `File` nodes with a `last_accessed` timestamp). The Waypointer's
/// "Recent Files" section reads the top-N by that timestamp.
///
/// Sourcing this from the graph instead of a local JSON file has two
/// concrete advantages:
///
///   1. **Cross-app:** captures file opens from any application, not
///      just Lunaris-shell-initiated launches. A file touched in the
///      terminal or via another app shows up here.
///   2. **Single source of truth:** aligns with Lunaris' "Knowledge
///      Graph as first-class infrastructure" principle — no
///      parallel mini-database for what the graph already tracks.
///
/// Graceful degradation: if the Knowledge daemon is down, the socket
/// is missing, or the query times out, we return an empty list. The
/// Waypointer frontend renders nothing (no "Recent Files" section),
/// which is the honest presentation of "we don't know right now".

use serde::{Deserialize, Serialize};

use crate::projects::graph_query_async;

/// One recently-accessed file returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    /// Absolute filesystem path.
    pub path: String,
    /// Unix ms of the most recent open, as stored by the graph
    /// promotion pipeline.
    pub last_accessed: i64,
}

/// Short TTL cache: Waypointer open → one graph round-trip per 5s
/// window regardless of how many times the user re-opens. The graph
/// is eventually consistent within seconds anyway (promotion runs on
/// a 30s cycle), so 5s cache introduces no perceivable staleness.
const CACHE_TTL_MS: i64 = 5_000;

struct CachedList {
    fetched_at: i64,
    files: Vec<RecentFile>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

static CACHE: std::sync::OnceLock<tokio::sync::Mutex<Option<CachedList>>> =
    std::sync::OnceLock::new();

fn cache() -> &'static tokio::sync::Mutex<Option<CachedList>> {
    CACHE.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Parse pipe-delimited rows returned by the Knowledge daemon.
///
/// Format:
/// ```text
/// f.path|f.last_accessed
/// /home/tim/docs/report.md|1713567890123
/// /home/tim/Projects/lunaris-sys/CLAUDE.md|1713567123456
/// ```
fn parse_rows(raw: &str) -> Vec<RecentFile> {
    if raw.trim().is_empty() || raw.starts_with("ERROR") {
        return Vec::new();
    }
    let mut lines = raw.lines();
    lines.next(); // Skip header row.

    let mut out = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() < 2 {
            continue;
        }
        let path = cols[0].trim();
        if path.is_empty() {
            continue;
        }
        let last_accessed: i64 = cols[1].trim().parse().unwrap_or(0);
        out.push(RecentFile {
            path: path.to_string(),
            last_accessed,
        });
    }
    out
}

/// Return the top-N recently-opened files from the Knowledge Graph.
/// Empty list on any failure (daemon down, socket missing, timeout,
/// unparseable response) — never errors, never blocks the UI.
#[tauri::command]
pub async fn get_recent_files(limit: u32) -> Vec<RecentFile> {
    let limit = limit.max(1).min(50);

    // Cache check.
    {
        let guard = cache().lock().await;
        if let Some(entry) = guard.as_ref() {
            if now_ms() - entry.fetched_at < CACHE_TTL_MS {
                return entry.files.iter().take(limit as usize).cloned().collect();
            }
        }
    }

    let cypher = format!(
        "MATCH (f:File) WHERE f.last_accessed IS NOT NULL \
         RETURN f.path, f.last_accessed \
         ORDER BY f.last_accessed DESC \
         LIMIT {limit}"
    );

    let raw = match graph_query_async(cypher).await {
        Ok(r) => r,
        Err(e) => {
            log::debug!("recent_files: graph query failed: {e}");
            return Vec::new();
        }
    };

    let files = parse_rows(&raw);
    *cache().lock().await = Some(CachedList {
        fetched_at: now_ms(),
        files: files.clone(),
    });
    files
}

/// Open a file with the default application via `xdg-open`. Invoked
/// from the Waypointer when the user picks a row from Recent Files.
#[tauri::command]
pub fn open_recent_file(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("open_recent_file: empty path".into());
    }
    std::process::Command::new("xdg-open")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open {path}: {e}"))
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_result_returns_empty() {
        assert!(parse_rows("").is_empty());
        assert!(parse_rows("   ").is_empty());
        assert!(parse_rows("ERROR: something broke").is_empty());
    }

    #[test]
    fn parse_header_only_returns_empty() {
        assert!(parse_rows("f.path|f.last_accessed\n").is_empty());
    }

    #[test]
    fn parse_two_rows() {
        let raw = "f.path|f.last_accessed\n\
                   /home/tim/docs/a.md|1000\n\
                   /home/tim/docs/b.md|2000\n";
        let out = parse_rows(raw);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path, "/home/tim/docs/a.md");
        assert_eq!(out[0].last_accessed, 1000);
        assert_eq!(out[1].path, "/home/tim/docs/b.md");
        assert_eq!(out[1].last_accessed, 2000);
    }

    #[test]
    fn parse_skips_empty_path() {
        let raw = "f.path|f.last_accessed\n\
                   |1000\n\
                   /home/tim/valid.md|2000\n";
        let out = parse_rows(raw);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, "/home/tim/valid.md");
    }

    #[test]
    fn parse_handles_unparseable_timestamp() {
        let raw = "f.path|f.last_accessed\n\
                   /home/tim/x.md|NaN\n";
        let out = parse_rows(raw);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].last_accessed, 0);
    }
}
