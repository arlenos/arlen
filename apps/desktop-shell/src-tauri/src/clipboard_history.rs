/// Clipboard history: in-memory ring buffer fed by `wl-paste --watch`.
///
/// Privacy-first design: opt-in via `~/.config/lunaris/shell.toml`,
/// text-only (no images, no passwords), no disk persistence. When the
/// shell process exits, the history is gone. This matches the threat
/// model Lunaris targets: "don't leak what the user just copied" wins
/// over "preserve history across reboots".
///
/// Filter chain (applied in order; any miss drops the entry):
///
/// 1. Non-empty after trim
/// 2. Not identical to the current head (dedup)
/// 3. Source app not in the blocklist (keepassxc, bitwarden, 1password)
/// 4. Shannon entropy < 4.5 bits/char OR length >= 64 chars (not a password)
/// 5. Length capped at 10 KB (truncate with marker, still store)
///
/// Source-app attribution: we don't correlate with `clipboard.copy`
/// Event Bus events because the compositor currently ships them with
/// empty payloads (mime-type only, no app id). Instead we read the
/// currently-focused window from `WindowList` at the instant
/// `wl-paste --watch` fires. That's a reasonable proxy — the focused
/// window at copy-time is typically the app that initiated the copy.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::wayland_client::WindowList;

/// Max entries kept in-memory.
pub const MAX_ENTRIES: usize = 30;
/// Max UTF-8 bytes per entry. Longer content is truncated + marker.
pub const MAX_ENTRY_BYTES: usize = 10 * 1024;
/// Password-heuristic threshold: below this length AND above the
/// entropy floor triggers the "likely password" skip.
pub const PASSWORD_LEN_THRESHOLD: usize = 64;
/// Shannon entropy (bits/char) above which a short, whitespace-free
/// string is flagged as password-like. A 16-char random ASCII string
/// with all unique characters sits at log2(16) = 4.0 bits/char, and
/// typical human passwords land around 3.5-4.0. Setting the threshold
/// at 3.5 catches the random-looking case while leaving long English
/// words (entropy ~3.0 and below) alone. Combined with the length
/// cutoff (<64) and the whitespace exclusion, false positives on
/// ordinary prose are rare.
pub const PASSWORD_ENTROPY_THRESHOLD: f32 = 3.5;
/// App ids that the clipboard watcher refuses to ever record content
/// from. Matched case-insensitive on prefix so "keepassxc.KeePassXC"
/// is just as blocked as "keepassxc".
pub const BLOCKED_APPS: &[&str] = &[
    "keepassxc", "bitwarden", "1password", "onepassword", "keeweb",
    "pass", "gnome-keyring", "kwalletmanager",
];

/// One clipboard entry as surfaced to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEntry {
    /// Monotonic id, used for delete.
    pub id: u64,
    /// Entry content (truncated to `MAX_ENTRY_BYTES`).
    pub content: String,
    /// Unix ms when captured.
    pub timestamp_ms: i64,
    /// App id inferred from the focused window at capture time.
    /// Empty when no focused window was available.
    pub source_app_id: String,
    /// Always "text/plain" for now — we don't record anything else.
    pub mime: String,
}

/// Shared clipboard state.
pub struct ClipboardHistory {
    entries: Mutex<VecDeque<ClipboardEntry>>,
    next_id: AtomicU64,
    /// Whether the watcher is active. Set from shell.toml at startup.
    enabled: AtomicBool,
}

impl ClipboardHistory {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)),
            next_id: AtomicU64::new(1),
            enabled: AtomicBool::new(false),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Flip the enabled flag. Called by `start()` after reading
    /// `shell.toml`, and by tests to force-enable. Kept `pub(crate)`
    /// so the Settings app can eventually toggle live.
    pub(crate) fn set_enabled(&self, v: bool) {
        self.enabled.store(v, Ordering::Relaxed);
    }

    /// Snapshot for frontend/plugin consumption. Most-recent first.
    pub fn snapshot(&self) -> Vec<ClipboardEntry> {
        self.entries
            .lock()
            .ok()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Substring filter over current entries (case-insensitive). Empty
    /// filter returns everything in snapshot order.
    pub fn filter(&self, needle: &str) -> Vec<ClipboardEntry> {
        let needle = needle.trim().to_lowercase();
        let all = self.snapshot();
        if needle.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|e| e.content.to_lowercase().contains(&needle))
            .collect()
    }

    /// Lookup a single entry by id (for `execute` after the user picks).
    pub fn find(&self, id: u64) -> Option<ClipboardEntry> {
        self.entries
            .lock()
            .ok()
            .and_then(|g| g.iter().find(|e| e.id == id).cloned())
    }

    /// Append a new entry after filter/dedup. Returns the stored entry
    /// if it was accepted, `None` if filtered out. `pub(crate)` so the
    /// plugin tests can seed the ring without spawning a watcher.
    pub(crate) fn push(&self, content: String, source_app_id: String) -> Option<ClipboardEntry> {
        let content = truncate_to_bytes(&content, MAX_ENTRY_BYTES);
        if content.trim().is_empty() {
            return None;
        }
        if is_blocked_app(&source_app_id) {
            return None;
        }
        if looks_like_password(&content) {
            return None;
        }

        let mut entries = self.entries.lock().ok()?;
        // Dedup: same content as the head is almost always a re-copy
        // of the same thing (mouse-highlight flicker, paste-reselect,
        // Cmd-C twice). Don't surface duplicates.
        if let Some(head) = entries.front() {
            if head.content == content {
                return None;
            }
        }

        let entry = ClipboardEntry {
            id: self.next_id.fetch_add(1, Ordering::SeqCst),
            content,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            source_app_id,
            mime: "text/plain".into(),
        };
        entries.push_front(entry.clone());
        while entries.len() > MAX_ENTRIES {
            entries.pop_back();
        }
        Some(entry)
    }

    /// Remove one entry by id. No-op if the id is unknown.
    fn delete(&self, id: u64) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|e| e.id != id);
        }
    }

    fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

impl Default for ClipboardHistory {
    fn default() -> Self {
        Self::new()
    }
}

/// Tauri-managed clipboard history handle.
pub type ClipboardHistoryState = Arc<ClipboardHistory>;

/// Truncate UTF-8 text to `max_bytes`, appending a marker if cut.
/// Respects char boundaries so we never produce invalid UTF-8.
pub fn truncate_to_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let marker = "… [truncated]";
    let budget = max_bytes.saturating_sub(marker.len());
    let mut end = budget;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str(marker);
    out
}

/// Case-insensitive prefix check against BLOCKED_APPS.
pub fn is_blocked_app(app_id: &str) -> bool {
    let a = app_id.to_lowercase();
    if a.is_empty() {
        return false;
    }
    BLOCKED_APPS.iter().any(|b| {
        let bl = b.to_lowercase();
        a.contains(&bl)
    })
}

/// Heuristic: short, high-entropy strings probably contain secrets.
/// Short on its own isn't enough (e.g. "yes" has low entropy, boring
/// text); high entropy on its own isn't enough (e.g. long essays can
/// hit 4.5+ bits/char in Latin). Both together is the password
/// signal. Threshold values match the plan doc.
pub fn looks_like_password(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() >= PASSWORD_LEN_THRESHOLD {
        return false;
    }
    // Space-heavy strings aren't passwords even if entropy is high.
    if trimmed.contains(' ') || trimmed.contains('\t') || trimmed.contains('\n') {
        return false;
    }
    shannon_entropy(trimmed) > PASSWORD_ENTROPY_THRESHOLD
}

/// Shannon entropy in bits/char.
pub fn shannon_entropy(s: &str) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts: HashMap<char, u32> = HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0) += 1;
    }
    let len = s.chars().count() as f32;
    counts
        .values()
        .map(|&c| {
            let p = c as f32 / len;
            -p * p.log2()
        })
        .sum()
}

/// Read `~/.config/lunaris/shell.toml` and return whether the
/// `[clipboard]` section has `enabled = true`. Missing file, missing
/// section, or parse error all default to `false`. This is the
/// privacy contract: clipboard history is inert unless the user has
/// explicitly opted in.
pub fn read_enabled_from_shell_toml() -> bool {
    let Some(path) = dirs::config_dir().map(|p| p.join("lunaris/shell.toml")) else {
        return false;
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(doc) = content.parse::<toml::Value>() else {
        return false;
    };
    doc.get("clipboard")
        .and_then(|c| c.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Start the wl-paste watcher on a dedicated thread.
///
/// If clipboard history is disabled in `shell.toml`, we return
/// without spawning anything. If `wl-paste` is missing, we log and
/// return — the user's distro ships without the `wl-clipboard`
/// package and that's fine, the rest of the shell still works.
///
/// Otherwise we spawn `wl-paste --watch` with a shell one-liner that
/// null-separates each clipboard change, read the pipe, and feed each
/// entry through the filter chain. The watcher auto-restarts after a
/// 1s backoff if the subprocess ever exits (normally it doesn't).
pub fn start(app: AppHandle, history: ClipboardHistoryState, window_list: WindowList) {
    let enabled = read_enabled_from_shell_toml();
    history.set_enabled(enabled);
    if !enabled {
        log::info!("clipboard_history: disabled (opt-in via shell.toml [clipboard] enabled=true)");
        return;
    }
    if !wl_paste_available() {
        log::warn!("clipboard_history: wl-paste not found in PATH, watcher disabled");
        history.set_enabled(false);
        return;
    }

    log::info!("clipboard_history: starting wl-paste watcher");

    std::thread::Builder::new()
        .name("clipboard-watcher".into())
        .spawn(move || {
            loop {
                if let Err(e) = run_watcher(&app, &history, &window_list) {
                    log::warn!("clipboard_history: watcher exited: {e}, retrying in 1s");
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("spawn clipboard-watcher thread");
}

fn wl_paste_available() -> bool {
    Command::new("wl-paste")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_watcher(
    app: &AppHandle,
    history: &ClipboardHistoryState,
    window_list: &WindowList,
) -> Result<(), String> {
    // `wl-paste --watch <cmd>` runs <cmd> on every clipboard change.
    // The sh one-liner prints current text content (if any) followed
    // by a NUL, so Rust can framing-split stdout without worrying
    // about newlines inside clipboard entries.
    let mut child = Command::new("wl-paste")
        .arg("--watch")
        .arg("sh")
        .arg("-c")
        .arg("wl-paste -t text/plain -n 2>/dev/null; printf '\\0'")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn wl-paste: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "wl-paste stdout pipe missing".to_string())?;

    let mut reader = BufReader::new(stdout);
    let mut buf: Vec<u8> = Vec::with_capacity(4096);

    loop {
        buf.clear();
        let bytes_read = reader
            .read_until(0, &mut buf)
            .map_err(|e| format!("read: {e}"))?;
        if bytes_read == 0 {
            return Err("wl-paste stdout EOF (subprocess exited)".into());
        }
        // Drop trailing NUL.
        if buf.last() == Some(&0) {
            buf.pop();
        }
        let content = match String::from_utf8(buf.clone()) {
            Ok(s) => s,
            Err(_) => continue, // binary content — skip
        };
        if content.is_empty() {
            continue; // non-text selection; wl-paste printed nothing
        }
        let source_app_id = focused_app_id(window_list).unwrap_or_default();
        if let Some(entry) = history.push(content, source_app_id) {
            // Frontend can watch this event to refresh the panel in
            // real time. Omitted from the MVP frontend but cheap to
            // emit regardless.
            let _ = app.emit("lunaris://clipboard-added", &entry);
        }
    }
}

/// Snapshot the focused window's app id from the shared WindowList.
/// Multiple windows can be active simultaneously on multi-output
/// setups; we pick the first because Wayland doesn't expose a
/// globally-unique "this is the active one right now" state.
fn focused_app_id(window_list: &WindowList) -> Option<String> {
    let list = window_list.lock().ok()?;
    for win in list.iter() {
        if win.active {
            return Some(win.app_id.clone());
        }
    }
    None
}

// ── Tauri commands ───────────────────────────────────────────────────

/// Full snapshot. Frontend calls this on popover open; per-keystroke
/// filtering is done in the plugin via `waypointer_search_plugin`.
#[tauri::command]
pub fn clipboard_get_entries(
    state: State<'_, ClipboardHistoryState>,
) -> Vec<ClipboardEntry> {
    state.snapshot()
}

/// Delete a single entry. Safe to call with an unknown id.
#[tauri::command]
pub fn clipboard_delete_entry(
    id: u64,
    state: State<'_, ClipboardHistoryState>,
    app: AppHandle,
) {
    state.delete(id);
    let _ = app.emit("lunaris://clipboard-changed", ());
}

/// Drop the entire history. Irreversible; no confirmation here (the
/// UI is expected to confirm before invoking).
#[tauri::command]
pub fn clipboard_clear_all(
    state: State<'_, ClipboardHistoryState>,
    app: AppHandle,
) {
    state.clear();
    let _ = app.emit("lunaris://clipboard-changed", ());
}

/// Report whether the watcher is active. The frontend hides the
/// clipboard section entirely when disabled so users don't see an
/// empty panel and wonder where their clipboard history went.
#[tauri::command]
pub fn clipboard_is_enabled(state: State<'_, ClipboardHistoryState>) -> bool {
    state.is_enabled()
}

/// Copy one entry's content back to the clipboard via `wl-copy`.
/// Separate from the plugin's `execute()` so the frontend can invoke
/// it directly for the "click to paste" affordance without going
/// through the plugin manager round-trip.
#[tauri::command]
pub fn clipboard_copy_entry(
    id: u64,
    state: State<'_, ClipboardHistoryState>,
) -> Result<(), String> {
    let entry = state
        .find(id)
        .ok_or_else(|| format!("clipboard: unknown entry id {id}"))?;
    copy_via_wl_copy(&entry.content)
}

/// Spawn `wl-copy` with the entry's content piped on stdin. Detaches
/// stdio so we don't block on the child; wl-copy forks itself to
/// persist the selection and exits immediately.
pub fn copy_via_wl_copy(content: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn wl-copy: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("write wl-copy stdin: {e}"))?;
    }
    let _ = child.wait();
    Ok(())
}

/// Give lib.rs a clone-able handle so we can both `.manage()` it for
/// Tauri and pass it to the plugin constructor.
pub fn create_state() -> ClipboardHistoryState {
    Arc::new(ClipboardHistory::new())
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shannon_entropy_zero_for_single_char() {
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn shannon_entropy_rises_with_variety() {
        let low = shannon_entropy("aaaaaa");
        let high = shannon_entropy("xK7$pQzN9!");
        assert!(high > low);
        assert!(high > 3.0);
    }

    #[test]
    fn password_heuristic_blocks_short_high_entropy() {
        assert!(looks_like_password("xK7$pQzN9!aBcDeF"));
        assert!(looks_like_password("P@ssw0rd123!Xz9"));
    }

    #[test]
    fn password_heuristic_allows_plain_text() {
        assert!(!looks_like_password("hello world"));
        assert!(!looks_like_password("ok"));
        assert!(!looks_like_password("  lorem ipsum  "));
    }

    #[test]
    fn password_heuristic_allows_long_strings() {
        // Long strings are typically content, not secrets. A 64+ char
        // string with spaces/variety isn't going to be a password.
        let long = "This is a long paragraph of content that a user might copy out of a web page for later reference.";
        assert!(!looks_like_password(long));
    }

    #[test]
    fn password_heuristic_allows_multiline() {
        // If it has whitespace, it's content, not a password.
        assert!(!looks_like_password("line1\nline2"));
    }

    #[test]
    fn password_heuristic_ignores_empty() {
        assert!(!looks_like_password(""));
        assert!(!looks_like_password("   "));
    }

    #[test]
    fn blocked_app_exact_match() {
        assert!(is_blocked_app("keepassxc"));
        assert!(is_blocked_app("org.keepassxc.KeePassXC"));
        assert!(is_blocked_app("Bitwarden"));
        assert!(is_blocked_app("com.bitwarden.desktop"));
    }

    #[test]
    fn blocked_app_unknown_allowed() {
        assert!(!is_blocked_app("firefox"));
        assert!(!is_blocked_app(""));
        assert!(!is_blocked_app("code"));
    }

    #[test]
    fn truncate_preserves_short_strings() {
        let s = "hello";
        assert_eq!(truncate_to_bytes(s, 1024), "hello");
    }

    #[test]
    fn truncate_cuts_with_marker() {
        let s = "a".repeat(20_000);
        let out = truncate_to_bytes(&s, 100);
        assert!(out.len() <= 100);
        assert!(out.ends_with("… [truncated]"));
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // "ä" is 2 bytes. Truncating inside it must step back.
        let s = "aaaäaaa";
        let out = truncate_to_bytes(s, 4);
        // Must still be valid UTF-8 (no panic, no mojibake).
        assert!(out.is_char_boundary(out.len() - "… [truncated]".len()));
    }

    #[test]
    fn push_respects_max_entries() {
        let h = ClipboardHistory::new();
        for i in 0..(MAX_ENTRIES + 10) {
            h.push(format!("entry-{i}"), String::new());
        }
        let snap = h.snapshot();
        assert_eq!(snap.len(), MAX_ENTRIES);
        // Most recent is first.
        assert!(snap[0].content.starts_with(&format!("entry-{}", MAX_ENTRIES + 9)));
    }

    #[test]
    fn push_dedups_head() {
        let h = ClipboardHistory::new();
        assert!(h.push("hello".into(), "".into()).is_some());
        assert!(h.push("hello".into(), "".into()).is_none());
        assert_eq!(h.snapshot().len(), 1);
    }

    #[test]
    fn push_blocked_app_skipped() {
        let h = ClipboardHistory::new();
        assert!(h.push("anything".into(), "keepassxc".into()).is_none());
        assert!(h.snapshot().is_empty());
    }

    #[test]
    fn push_password_skipped() {
        let h = ClipboardHistory::new();
        assert!(h.push("xK7$pQzN9!aBcD".into(), "firefox".into()).is_none());
        assert!(h.snapshot().is_empty());
    }

    #[test]
    fn push_empty_skipped() {
        let h = ClipboardHistory::new();
        assert!(h.push("   ".into(), "".into()).is_none());
        assert!(h.push("".into(), "".into()).is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let h = ClipboardHistory::new();
        let e = h.push("hello".into(), "".into()).unwrap();
        assert_eq!(h.snapshot().len(), 1);
        h.delete(e.id);
        assert!(h.snapshot().is_empty());
    }

    #[test]
    fn delete_unknown_id_is_noop() {
        let h = ClipboardHistory::new();
        h.push("hello".into(), "".into());
        h.delete(9999);
        assert_eq!(h.snapshot().len(), 1);
    }

    #[test]
    fn clear_drops_all() {
        let h = ClipboardHistory::new();
        for i in 0..5 {
            h.push(format!("e{i}"), "".into());
        }
        h.clear();
        assert!(h.snapshot().is_empty());
    }

    #[test]
    fn filter_substring_case_insensitive() {
        let h = ClipboardHistory::new();
        h.push("hello world".into(), "".into());
        h.push("goodbye".into(), "".into());
        h.push("Hello there".into(), "".into());

        let r = h.filter("hello");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn filter_empty_returns_all() {
        let h = ClipboardHistory::new();
        h.push("a".into(), "".into());
        h.push("b".into(), "".into());
        assert_eq!(h.filter("").len(), 2);
        assert_eq!(h.filter("   ").len(), 2);
    }

    #[test]
    fn find_by_id() {
        let h = ClipboardHistory::new();
        let e = h.push("target".into(), "".into()).unwrap();
        let found = h.find(e.id).unwrap();
        assert_eq!(found.content, "target");
        assert!(h.find(9999).is_none());
    }

    #[test]
    fn truncation_integrated_in_push() {
        let h = ClipboardHistory::new();
        let big = "a".repeat(MAX_ENTRY_BYTES * 2);
        let e = h.push(big, "".into()).unwrap();
        assert!(e.content.len() <= MAX_ENTRY_BYTES);
        assert!(e.content.ends_with("… [truncated]"));
    }

    /// Both XDG_CONFIG_HOME-dependent tests share a mutex because
    /// they mutate the global process env; parallel test runs would
    /// otherwise see each other's values and flake unpredictably.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn enabled_defaults_to_false_without_config() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        // Point XDG at an empty dir. `dirs::config_dir()` will prefer
        // XDG_CONFIG_HOME over HOME-derived paths, so the
        // lunaris/shell.toml lookup returns nothing.
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        assert!(!read_enabled_from_shell_toml());
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn enabled_reads_shell_toml_section() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("lunaris");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("shell.toml"),
            "[clipboard]\nenabled = true\n",
        )
        .unwrap();
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        assert!(read_enabled_from_shell_toml());
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
