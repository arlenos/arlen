/// File search plugin: surfaces files from the Knowledge Graph.
///
/// The Lunaris graph tracks every `file.opened` event system-wide via
/// eBPF -> Event Bus -> Knowledge daemon, promoting them to `File`
/// nodes with `path`, `app_id`, and `last_accessed`. This plugin turns
/// that graph into a Waypointer section that Baloo/Spotlight cannot
/// replicate: queries like `project:lunaris` or `app:cursor` are free
/// because the graph already knows FILE_PART_OF and ACCESSED_BY
/// edges.
///
/// Design choices:
///
/// - **Sync `search` with `graph_query`**: matches `ProjectsPlugin`'s
///   pattern. The PluginManager's `search_plugin` is itself sync; the
///   frontend wraps it in `invoke()` so it lands on a Tauri worker
///   thread. A 200ms timeout is enforced via the socket read timeout
///   so a hung daemon can't stall the UI.
/// - **Query modes parsed in the plugin**: `project:<filter>`,
///   `app:<filter>`, or plain substring. No frontend-side prefix
///   dispatch; the frontend always routes every keystroke to this
///   plugin via the `search_plugin` bridge and lets Rust decide how to
///   interpret it.
/// - **Fetch-then-filter**: the Cypher pulls the top 200 recently-
///   accessed files (joined with their project) and scoring / filtering
///   happens in Rust. Keeps the daemon query simple and portable
///   between Ladybug / Kuzu versions without relying on `LOWER()` /
///   `CONTAINS` which have inconsistent availability.
/// - **5s TTL cache**: one round-trip per query-mode per 5s window,
///   identical pattern to `recent_files.rs`. Graph promotion runs on
///   a 30s cycle, so 5s is imperceptible staleness.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::waypointer_system::plugin::*;

/// Max files fetched from the graph per round-trip. The user sees at
/// most `max_results()` after scoring, but we need a larger candidate
/// pool so substring filtering finds matches even when the top-20
/// recent files don't contain the query.
const FETCH_LIMIT: usize = 200;

/// TTL for the query-result cache. Matches `recent_files.rs`.
const CACHE_TTL_MS: i64 = 5_000;

/// Hard ceiling on a graph round-trip from the plugin. Beyond this the
/// plugin returns empty rather than freezing the Waypointer.
const GRAPH_TIMEOUT_MS: u64 = 200;

pub struct FilesPlugin {
    cache: Mutex<Option<CachedQuery>>,
}

impl FilesPlugin {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(None),
        }
    }
}

impl Default for FilesPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// One cached query: its normalised key and the files we returned.
struct CachedQuery {
    key: String,
    fetched_at: i64,
    files: Vec<FileRow>,
}

/// One row from the graph response, before scoring.
#[derive(Debug, Clone)]
struct FileRow {
    path: String,
    app_id: String,
    last_accessed: i64,
    project_name: Option<String>,
}

/// Parsed query mode.
enum QueryMode<'a> {
    Plain(&'a str),
    Project(&'a str),
    App(&'a str),
}

impl<'a> QueryMode<'a> {
    fn parse(q: &'a str) -> Self {
        let trimmed = q.trim();
        // Strip exclusive `f:` / `file:` prefix — the frontend routes
        // every keystroke here via `search_plugin`, so the prefix is
        // purely user-facing ("show me only files").
        let stripped = trimmed
            .strip_prefix("file:")
            .or_else(|| trimmed.strip_prefix("f:"))
            .map(str::trim)
            .unwrap_or(trimmed);

        if let Some(rest) = stripped.strip_prefix("project:") {
            return QueryMode::Project(rest.trim());
        }
        if let Some(rest) = stripped.strip_prefix("app:") {
            return QueryMode::App(rest.trim());
        }
        QueryMode::Plain(stripped)
    }

    fn filter(&self) -> &str {
        match self {
            QueryMode::Plain(s) | QueryMode::Project(s) | QueryMode::App(s) => s,
        }
    }

    fn cache_key(&self) -> String {
        match self {
            QueryMode::Plain(s) => format!("plain:{}", s.to_lowercase()),
            QueryMode::Project(s) => format!("project:{}", s.to_lowercase()),
            QueryMode::App(s) => format!("app:{}", s.to_lowercase()),
        }
    }
}

impl WaypointerPlugin for FilesPlugin {
    fn id(&self) -> &str { "core.files" }
    fn name(&self) -> &str { "Files" }
    fn description(&self) -> &str {
        "Search files by name, by project, or by the app that accessed them — all from the Knowledge Graph."
    }
    fn priority(&self) -> u32 { 8 }
    fn max_results(&self) -> usize { 20 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let mode = QueryMode::parse(trimmed);
        let filter = mode.filter().to_lowercase();
        // Without a filter we don't show anything: recent-files already
        // surfaces the empty-query view from `recent_files.rs`, and
        // repeating the same top-20 here would just duplicate that
        // section.
        if filter.is_empty() {
            return Vec::new();
        }

        let key = mode.cache_key();
        let rows = match cached_rows(&self.cache, &key) {
            Some(r) => r,
            None => {
                let rows = fetch_rows();
                store_cache(&self.cache, key, rows.clone());
                rows
            }
        };

        score_and_rank(&rows, &mode, self.max_results())
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        let Action::Open { ref path } = result.action else {
            return Err(PluginError::ExecuteFailed(
                "files: expected Action::Open".into(),
            ));
        };

        if !path.exists() {
            return Err(PluginError::ExecuteFailed(format!(
                "file no longer exists: {}",
                path.display()
            )));
        }

        std::process::Command::new("xdg-open")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| PluginError::ExecuteFailed(format!("xdg-open: {e}")))
    }
}

/// Cache lookup. Returns `Some(rows)` when a fresh entry exists.
fn cached_rows(cache: &Mutex<Option<CachedQuery>>, key: &str) -> Option<Vec<FileRow>> {
    let guard = cache.lock().ok()?;
    let entry = guard.as_ref()?;
    if entry.key != key {
        return None;
    }
    if now_ms() - entry.fetched_at >= CACHE_TTL_MS {
        return None;
    }
    Some(entry.files.clone())
}

fn store_cache(cache: &Mutex<Option<CachedQuery>>, key: String, files: Vec<FileRow>) {
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedQuery {
            key,
            fetched_at: now_ms(),
            files,
        });
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Fetch the top-N recently-accessed files with their project label.
///
/// Returns empty on any failure (daemon down, socket missing, timeout,
/// parse error). Never panics. Graceful degradation is the whole point:
/// the Waypointer renders nothing when the graph is unavailable, which
/// is the honest representation of "we don't know right now."
fn fetch_rows() -> Vec<FileRow> {
    let cypher = format!(
        "MATCH (f:File) \
         WHERE f.last_accessed IS NOT NULL \
         OPTIONAL MATCH (f)-[:FILE_PART_OF]->(p:Project) \
         RETURN f.path, f.app_id, f.last_accessed, p.name \
         ORDER BY f.last_accessed DESC \
         LIMIT {FETCH_LIMIT}"
    );
    match graph_query_sync(&cypher) {
        Ok(raw) => parse_rows(&raw),
        Err(e) => {
            log::debug!("files plugin: graph query failed: {e}");
            Vec::new()
        }
    }
}

/// Knowledge daemon socket path (same fallback chain as `projects.rs`).
fn knowledge_socket_path() -> String {
    if let Ok(p) = std::env::var("LUNARIS_DAEMON_SOCKET") {
        return p;
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return format!("{xdg}/lunaris/knowledge.sock");
    }
    "/run/lunaris/knowledge.sock".to_string()
}

/// Send a Cypher query to the Knowledge Daemon with explicit read /
/// write timeouts. The sync `projects::graph_query` lacks these; we
/// don't call it here because a hung daemon would freeze the plugin
/// thread — which on hot queries (every keystroke) is the difference
/// between "a laggy Waypointer" and "a frozen one."
fn graph_query_sync(cypher: &str) -> Result<String, String> {
    let socket = knowledge_socket_path();
    let mut stream = UnixStream::connect(&socket).map_err(|e| format!("connect: {e}"))?;
    let timeout = Duration::from_millis(GRAPH_TIMEOUT_MS);
    stream.set_read_timeout(Some(timeout)).map_err(|e| format!("set_read_timeout: {e}"))?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| format!("set_write_timeout: {e}"))?;

    let query_bytes = cypher.as_bytes();
    let len = (query_bytes.len() as u32).to_be_bytes();
    stream.write_all(&len).map_err(|e| format!("write len: {e}"))?;
    stream.write_all(query_bytes).map_err(|e| format!("write query: {e}"))?;
    stream.flush().map_err(|e| format!("flush: {e}"))?;

    let mut resp_len = [0u8; 4];
    stream.read_exact(&mut resp_len).map_err(|e| format!("read len: {e}"))?;
    let resp_size = u32::from_be_bytes(resp_len) as usize;
    if resp_size > 4 * 1024 * 1024 {
        return Err(format!("response too large: {resp_size}"));
    }
    let mut buf = vec![0u8; resp_size];
    stream.read_exact(&mut buf).map_err(|e| format!("read body: {e}"))?;
    String::from_utf8(buf).map_err(|e| format!("utf8: {e}"))
}

/// Parse pipe-delimited rows: `f.path|f.app_id|f.last_accessed|p.name`.
/// Empty or missing project name is fine (OPTIONAL MATCH can return
/// NULL or empty string depending on Ladybug version).
fn parse_rows(raw: &str) -> Vec<FileRow> {
    if raw.trim().is_empty() || raw.starts_with("ERROR") {
        return Vec::new();
    }
    let mut lines = raw.lines();
    lines.next(); // header

    let mut out = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() < 3 {
            continue;
        }
        let path = cols[0].trim().to_string();
        if path.is_empty() {
            continue;
        }
        let app_id = cols[1].trim().to_string();
        let last_accessed: i64 = cols[2].trim().parse().unwrap_or(0);
        let project_name = cols
            .get(3)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && *s != "NULL")
            .map(str::to_string);

        out.push(FileRow {
            path,
            app_id,
            last_accessed,
            project_name,
        });
    }
    out
}

/// Score a row against the query and build a `SearchResult`. Inline
/// the filtering so one pass produces the ranked output.
fn score_and_rank(rows: &[FileRow], mode: &QueryMode<'_>, cap: usize) -> Vec<SearchResult> {
    let filter = mode.filter().to_lowercase();
    let now = now_ms();

    let mut scored: Vec<(f32, &FileRow)> = Vec::new();
    for row in rows {
        let score = match mode {
            QueryMode::Plain(_) => score_by_path(&row.path, &filter),
            QueryMode::Project(_) => score_by_project(row, &filter),
            QueryMode::App(_) => score_by_app(&row.app_id, &filter),
        };
        if let Some(s) = score {
            let bonus = recency_bonus(row.last_accessed, now);
            scored.push(((s + bonus).min(1.0), row));
        }
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .take(cap)
        .map(|(score, row)| build_result(row, score, now))
        .collect()
}

/// Score a path against a substring query. Matches basename first
/// (the part the user most likely types), then full path, then gives
/// up. Mirrors the Power-plugin shape: exact basename 1.0, prefix 0.8,
/// contains 0.5.
fn score_by_path(path: &str, filter: &str) -> Option<f32> {
    let basename = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let full = path.to_lowercase();

    if basename == filter {
        return Some(1.0);
    }
    if basename.starts_with(filter) {
        return Some(0.8);
    }
    if basename.contains(filter) {
        return Some(0.65);
    }
    if full.contains(filter) {
        return Some(0.5);
    }
    None
}

fn score_by_project(row: &FileRow, filter: &str) -> Option<f32> {
    let name = row.project_name.as_deref()?.to_lowercase();
    if filter.is_empty() {
        // `project:` with no filter: every project-tagged file qualifies.
        return Some(0.5);
    }
    if name == filter {
        return Some(1.0);
    }
    if name.starts_with(filter) {
        return Some(0.8);
    }
    if name.contains(filter) {
        return Some(0.6);
    }
    None
}

fn score_by_app(app_id: &str, filter: &str) -> Option<f32> {
    let id = app_id.to_lowercase();
    if filter.is_empty() {
        return Some(0.5);
    }
    if id == filter {
        return Some(1.0);
    }
    if id.starts_with(filter) {
        return Some(0.8);
    }
    if id.contains(filter) {
        return Some(0.6);
    }
    None
}

/// Recency bonus in [0, 0.1]. Same-day files get the full bump, files
/// older than a week round to zero. Keeps the bonus small enough that
/// an exact basename match beats any stale file.
fn recency_bonus(last_accessed: i64, now: i64) -> f32 {
    if last_accessed <= 0 {
        return 0.0;
    }
    const WEEK_MS: i64 = 7 * 86_400_000;
    let age = (now - last_accessed).max(0);
    if age >= WEEK_MS {
        return 0.0;
    }
    (1.0 - age as f32 / WEEK_MS as f32) * 0.1
}

fn build_result(row: &FileRow, relevance: f32, now: i64) -> SearchResult {
    let basename = Path::new(&row.path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| row.path.clone());

    let description = build_description(row, now);
    let icon = icon_for_path(&row.path);

    SearchResult {
        id: format!("file-{}", row.path),
        title: basename,
        description: Some(description),
        icon: Some(icon.into()),
        relevance,
        action: Action::Open { path: row.path.clone().into() },
        plugin_id: String::new(),
    }
}

fn build_description(row: &FileRow, now: i64) -> String {
    let rel = relative_time(row.last_accessed, now);
    match &row.project_name {
        Some(name) => format!("Project: {name} · {rel} · {}", row.path),
        None => format!("{rel} · {}", row.path),
    }
}

/// Human-readable relative time: "5 min ago", "2 days ago", …
fn relative_time(timestamp_ms: i64, now_ms: i64) -> String {
    if timestamp_ms <= 0 {
        return "unknown".into();
    }
    let diff = (now_ms - timestamp_ms).max(0);
    let seconds = diff / 1000;
    if seconds < 60 {
        return "just now".into();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} h ago");
    }
    let days = hours / 24;
    if days < 30 {
        return format!("{days} days ago");
    }
    let months = days / 30;
    format!("{months} mo ago")
}

/// Pick a lucide icon name based on file extension. The frontend maps
/// these to the corresponding lucide-svelte component so we don't have
/// to round-trip SVG data.
fn icon_for_path(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" | "ts" | "js" | "py" | "go" | "c" | "cpp" | "h" | "hpp" | "svelte" | "vue" | "tsx" | "jsx" | "java" | "rb" | "php" | "sh" | "zsh" | "bash" => "file-code",
        "md" | "txt" | "rst" | "adoc" | "org" => "file-text",
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" => "file-cog",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" => "file-image",
        "pdf" => "file-text",
        "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" => "file-archive",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => "file-audio",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "file-video",
        _ => "file",
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_row(path: &str, app_id: &str, age_ms: i64, project: Option<&str>) -> FileRow {
        FileRow {
            path: path.into(),
            app_id: app_id.into(),
            last_accessed: now_ms() - age_ms,
            project_name: project.map(String::from),
        }
    }

    #[test]
    fn parse_mode_plain() {
        let m = QueryMode::parse("waypointer");
        assert!(matches!(m, QueryMode::Plain("waypointer")));
    }

    #[test]
    fn parse_mode_strips_f_prefix() {
        let m = QueryMode::parse("f:test");
        assert!(matches!(m, QueryMode::Plain("test")));
        let m = QueryMode::parse("file:test");
        assert!(matches!(m, QueryMode::Plain("test")));
    }

    #[test]
    fn parse_mode_project() {
        let m = QueryMode::parse("project:lunaris");
        assert!(matches!(m, QueryMode::Project("lunaris")));
    }

    #[test]
    fn parse_mode_app() {
        let m = QueryMode::parse("app:cursor");
        assert!(matches!(m, QueryMode::App("cursor")));
    }

    #[test]
    fn parse_mode_prefixed_project() {
        // `f:project:X` should still recognise the sub-mode.
        let m = QueryMode::parse("f:project:lunaris");
        assert!(matches!(m, QueryMode::Project("lunaris")));
    }

    #[test]
    fn score_exact_basename_wins() {
        let exact = score_by_path("/home/tim/waypointer.ts", "waypointer.ts").unwrap();
        let prefix = score_by_path("/home/tim/waypointer.ts", "waypoint").unwrap();
        assert!(exact > prefix);
        assert!((exact - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn score_path_hit_below_basename_hit() {
        // "src" appears in path but not in basename.
        let path_only = score_by_path("/home/tim/src/main.rs", "src").unwrap();
        let basename_hit = score_by_path("/home/tim/main.rs", "main").unwrap();
        assert!(basename_hit > path_only);
    }

    #[test]
    fn score_no_match_returns_none() {
        assert!(score_by_path("/home/tim/doc.md", "xyz").is_none());
    }

    #[test]
    fn score_by_project_requires_project() {
        let row = mk_row("/a", "app", 1000, None);
        assert!(score_by_project(&row, "lunaris").is_none());
    }

    #[test]
    fn score_by_project_matches() {
        let row = mk_row("/a", "app", 1000, Some("Lunaris"));
        assert!(score_by_project(&row, "lunaris").is_some());
        assert!(score_by_project(&row, "lun").is_some());
        assert!(score_by_project(&row, "xyz").is_none());
    }

    #[test]
    fn recency_bonus_fresh_is_max() {
        let now = now_ms();
        let bonus = recency_bonus(now, now);
        assert!((bonus - 0.1).abs() < 0.001);
    }

    #[test]
    fn recency_bonus_ancient_is_zero() {
        let now = now_ms();
        let ancient = now - 30 * 86_400_000; // 30 days
        assert_eq!(recency_bonus(ancient, now), 0.0);
    }

    #[test]
    fn recency_bonus_invalid_timestamp_is_zero() {
        assert_eq!(recency_bonus(0, now_ms()), 0.0);
        assert_eq!(recency_bonus(-1, now_ms()), 0.0);
    }

    #[test]
    fn score_and_rank_sorts_by_score_then_recency() {
        let rows = vec![
            // Same filter match but older.
            mk_row("/a/waypointer.ts", "app.a", 7 * 86_400_000, None),
            // Fresher: recency bonus should lift it above the older one.
            mk_row("/b/waypointer.ts", "app.b", 1_000, None),
        ];
        let mode = QueryMode::Plain("waypointer.ts");
        let out = score_and_rank(&rows, &mode, 5);
        assert_eq!(out.len(), 2);
        assert!(out[0].relevance >= out[1].relevance);
        assert!(out[0].title.contains("waypointer.ts"));
    }

    #[test]
    fn score_and_rank_respects_cap() {
        let rows: Vec<FileRow> = (0..30)
            .map(|i| mk_row(&format!("/x/note-{i}.md"), "app", i * 1000, None))
            .collect();
        let mode = QueryMode::Plain("note");
        let out = score_and_rank(&rows, &mode, 5);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn parse_rows_handles_missing_project() {
        let raw = "f.path|f.app_id|f.last_accessed|p.name\n\
                   /a/x.md|app|1000|\n\
                   /b/y.md|app|2000|Lunaris\n";
        let out = parse_rows(raw);
        assert_eq!(out.len(), 2);
        assert!(out[0].project_name.is_none());
        assert_eq!(out[1].project_name.as_deref(), Some("Lunaris"));
    }

    #[test]
    fn parse_rows_handles_null_token() {
        // Some Ladybug versions print NULL for missing values.
        let raw = "f.path|f.app_id|f.last_accessed|p.name\n\
                   /a/x.md|app|1000|NULL\n";
        let out = parse_rows(raw);
        assert_eq!(out.len(), 1);
        assert!(out[0].project_name.is_none());
    }

    #[test]
    fn parse_rows_skips_error_lines() {
        assert!(parse_rows("ERROR something").is_empty());
        assert!(parse_rows("").is_empty());
        assert!(parse_rows("   ").is_empty());
    }

    #[test]
    fn parse_rows_skips_malformed_row() {
        let raw = "f.path|f.app_id|f.last_accessed|p.name\n\
                   too|few\n\
                   /good|app|100|\n";
        let out = parse_rows(raw);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, "/good");
    }

    #[test]
    fn icon_for_rust_file() {
        assert_eq!(icon_for_path("/a/main.rs"), "file-code");
        assert_eq!(icon_for_path("/a/note.md"), "file-text");
        assert_eq!(icon_for_path("/a/image.png"), "file-image");
        assert_eq!(icon_for_path("/a/data.json"), "file-cog");
        assert_eq!(icon_for_path("/a/archive.tar.gz"), "file-archive");
        assert_eq!(icon_for_path("/a/video.mp4"), "file-video");
        assert_eq!(icon_for_path("/a/song.mp3"), "file-audio");
        assert_eq!(icon_for_path("/a/unknown"), "file");
    }

    #[test]
    fn relative_time_buckets() {
        let now = now_ms();
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - 30 * 1000, now), "just now");
        assert_eq!(relative_time(now - 5 * 60 * 1000, now), "5 min ago");
        assert_eq!(relative_time(now - 3 * 3600 * 1000, now), "3 h ago");
        assert_eq!(relative_time(now - 2 * 86_400_000, now), "2 days ago");
        assert_eq!(relative_time(now - 60 * 86_400_000, now), "2 mo ago");
    }

    #[test]
    fn relative_time_unknown_on_zero() {
        let now = now_ms();
        assert_eq!(relative_time(0, now), "unknown");
        assert_eq!(relative_time(-1, now), "unknown");
    }

    #[test]
    fn plugin_empty_query_returns_empty() {
        let p = FilesPlugin::new();
        assert!(p.search("").is_empty());
        assert!(p.search("   ").is_empty());
    }

    #[test]
    fn plugin_graceful_when_daemon_unreachable() {
        // Point at a socket that can't be connected to. Plugin must
        // return empty, not panic, not error.
        std::env::set_var("LUNARIS_DAEMON_SOCKET", "/tmp/nonexistent-lunaris-test-socket");
        let p = FilesPlugin::new();
        let r = p.search("test");
        // Either empty (expected) or whatever is in a prior cache —
        // we can't assert emptiness strictly if another test populated
        // the cache first, but we CAN assert no panic.
        let _ = r;
        std::env::remove_var("LUNARIS_DAEMON_SOCKET");
    }

    #[test]
    fn execute_rejects_wrong_action_variant() {
        let p = FilesPlugin::new();
        let r = SearchResult {
            id: "file-x".into(),
            title: "x".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Copy { text: "hi".into() },
            plugin_id: "core.files".into(),
        };
        assert!(p.execute(&r).is_err());
    }

    #[test]
    fn execute_rejects_missing_file() {
        let p = FilesPlugin::new();
        let r = SearchResult {
            id: "file-x".into(),
            title: "x".into(),
            description: None,
            icon: None,
            relevance: 1.0,
            action: Action::Open { path: "/nonexistent/lunaris-test-path/x.md".into() },
            plugin_id: "core.files".into(),
        };
        assert!(p.execute(&r).is_err());
    }

    #[test]
    fn cache_hit_returns_stored_rows() {
        let cache = Mutex::new(None);
        let rows = vec![mk_row("/a/x.md", "app", 1000, None)];
        store_cache(&cache, "plain:test".into(), rows.clone());
        let hit = cached_rows(&cache, "plain:test");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().len(), 1);
    }

    #[test]
    fn cache_miss_on_different_key() {
        let cache = Mutex::new(None);
        store_cache(&cache, "plain:test".into(), vec![]);
        assert!(cached_rows(&cache, "plain:other").is_none());
    }

    #[test]
    fn cache_miss_when_stale() {
        let cache = Mutex::new(None);
        // Manually insert a stale entry.
        {
            let mut guard = cache.lock().unwrap();
            *guard = Some(CachedQuery {
                key: "plain:test".into(),
                fetched_at: now_ms() - CACHE_TTL_MS - 1_000,
                files: vec![],
            });
        }
        assert!(cached_rows(&cache, "plain:test").is_none());
    }

    #[test]
    fn plugin_metadata() {
        let p = FilesPlugin::new();
        assert_eq!(p.id(), "core.files");
        assert_eq!(p.prefix(), None);
        assert_eq!(p.max_results(), 20);
        assert_eq!(p.priority(), 8);
    }
}
