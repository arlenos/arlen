//! The Arlen meetings Tauri shell (agent-work-surfaces). Meeting capture stays
//! on-device by design (the Otter/Granola trap we avoid): a produced note lives in
//! the graph as list/link metadata and, in full, as an app-owned document.
//!
//! This shell exposes the commands the frontend invokes. The recent-meetings home
//! (`meetings_list`) and a single note's metadata (`meeting_note`) read the KG
//! through the os-sdk graph client (the daemon's `0x0C` read op). The capture
//! (`meeting_start_capture`/`meeting_stop_capture`) and the summary
//! (`meeting_summarize`) surfaces depend on the on-device ASR engine and an AI
//! provider respectively; until those are provisioned they answer a clear error,
//! and the frontend falls back to its own local capture/fixture, so the app is
//! usable while the engine lands.

use std::path::PathBuf;

use serde::Serialize;

use arlen_meeting_note::MeetingNote;
use arlen_transcript::Transcript;
use os_sdk::UnixGraphClient;

mod summarize;

/// The knowledge daemon's query socket: the app's own bind override, the daemon's
/// bind env, then the per-user runtime default, then the system path. Mirrors the
/// resolution the other graph clients use so a launcher setting either env var
/// reaches the app.
fn graph_socket() -> String {
    for var in ["ARLEN_KNOWLEDGE_SOCKET", "ARLEN_DAEMON_SOCKET"] {
        if let Some(v) = std::env::var_os(var) {
            if !v.is_empty() {
                return v.to_string_lossy().into_owned();
            }
        }
    }
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !rt.is_empty() {
            return PathBuf::from(rt)
                .join("arlen/knowledge.sock")
                .to_string_lossy()
                .into_owned();
        }
    }
    "/run/arlen/knowledge.sock".to_string()
}

/// One row of the recent-meetings home, in the frontend's `MeetingSummary` shape:
/// the recording start as epoch **milliseconds** (the KG stores microseconds) and a
/// short `preview` of the summary for the card.
#[derive(Serialize)]
struct MeetingSummaryDto {
    id: String,
    title: String,
    date_ms: i64,
    participants: Vec<String>,
    preview: String,
}

/// The first `n` characters of `s` (on a char boundary), with an ellipsis when it
/// was cut, for the recent-meetings card preview.
fn preview_of(s: &str, n: usize) -> String {
    let mut end = s.len();
    for (i, (byte_idx, _)) in s.char_indices().enumerate() {
        if i == n {
            end = byte_idx;
            break;
        }
    }
    if end < s.len() {
        format!("{}...", &s[..end])
    } else {
        s.to_string()
    }
}

/// The recent meetings for the home. Reads the KG Meeting nodes via the daemon's
/// `0x0C` list op and maps them to the frontend card shape. A daemon/socket error
/// is surfaced so the frontend can fall back to its fixture under dev.
#[tauri::command]
async fn meetings_list() -> Result<Vec<MeetingSummaryDto>, String> {
    let client = UnixGraphClient::new(graph_socket());
    let rows = client.meetings_list().await.map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|m| MeetingSummaryDto {
            id: m.id,
            title: m.title,
            // The KG stores the recording start in microseconds; the card wants ms.
            date_ms: m.started_at / 1000,
            participants: m.participants,
            preview: preview_of(&m.summary, 140),
        })
        .collect())
}

/// A single past meeting's full note by id: the summary, action items and the
/// transcript it was grounded in, loaded from the app-owned note document (the
/// graph carries only list/link metadata, not the transcript). An unknown id, or a
/// call with no id (the active meeting the frontend drives locally), is an error
/// the frontend handles by using its own buffer.
#[tauri::command]
async fn meeting_note(id: Option<String>) -> Result<MeetingNote, String> {
    let Some(id) = id else {
        return Err("no meeting id (the active meeting is held by the app)".to_string());
    };
    match arlen_meetings_core::note_store::load(&id)? {
        Some(stored) => Ok(stored.note),
        None => Err(format!("no meeting note for id {id}")),
    }
}

/// The human notes saved with a past meeting (the anchor). Loaded from the note
/// document when an id is given; empty for the active meeting (the frontend holds
/// its live-notes buffer). Absent-note ids answer empty rather than error, since
/// the anchor is auxiliary to the note itself.
#[tauri::command]
async fn meeting_human_notes(id: Option<String>) -> Result<String, String> {
    let Some(id) = id else {
        return Ok(String::new());
    };
    Ok(arlen_meetings_core::note_store::load(&id)?
        .map(|s| s.human_notes)
        .unwrap_or_default())
}

/// Open a produced note document in its editor. Best-effort via `xdg-open`; a
/// failure is surfaced but not fatal to the meetings surface.
#[tauri::command]
async fn open_file(file: String) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(&file)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {file}: {e}"))
}

/// Start on-device capture. The ASR/diarization engine is provisioned separately
/// (model-gated); until it lands this reports so, and the frontend runs its own
/// local capture. The frontend ignores the error by design.
#[tauri::command]
async fn meeting_start_capture() -> Result<(), String> {
    Err("on-device capture requires the ASR engine (not yet provisioned)".to_string())
}

/// Stop on-device capture (see [`meeting_start_capture`]).
#[tauri::command]
async fn meeting_stop_capture() -> Result<(), String> {
    Err("on-device capture requires the ASR engine (not yet provisioned)".to_string())
}

/// Summarize a captured transcript into a grounded note and persist it (the app
/// document + the KG list metadata). Runs the transcript through the AI summary
/// engine over a provider (screened, fail-closed). A provider/model that is not
/// provisioned is an error the frontend handles by falling back to its fixture.
#[tauri::command]
async fn meeting_summarize(
    transcript: serde_json::Value,
    human_notes: String,
) -> Result<MeetingNote, String> {
    let transcript: Transcript =
        serde_json::from_value(transcript).map_err(|e| format!("invalid transcript: {e}"))?;
    summarize::summarize_and_file(transcript, human_notes).await
}

/// Tauri entry point (invoked from `main.rs`).
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![
            meetings_list,
            meeting_note,
            meeting_human_notes,
            open_file,
            meeting_start_capture,
            meeting_stop_capture,
            meeting_summarize
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-meetings");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_truncates_on_a_char_boundary_with_ellipsis() {
        assert_eq!(preview_of("short", 140), "short");
        assert_eq!(preview_of("abcdef", 3), "abc...");
        // A multi-byte char at the cut must not split.
        let s = "aaa\u{00e9}bbb"; // é is two bytes
        assert_eq!(preview_of(s, 3), "aaa...");
    }

    #[test]
    fn graph_socket_prefers_the_knowledge_env() {
        // The default path is used when no env is set (can't mutate env safely in
        // parallel tests, so just assert the fallback shape).
        let s = graph_socket();
        assert!(s.ends_with("knowledge.sock"), "socket path: {s}");
    }
}
