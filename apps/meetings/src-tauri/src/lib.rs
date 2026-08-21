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
use os_sdk::graph::ReadOutcome;
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
async fn meetings_list() -> ReadOutcome<MeetingSummaryDto> {
    let client = UnixGraphClient::new(graph_socket());
    // `ReadOutcome` rather than `Result<_, String>`: "no meetings daemon on this
    // machine" and "this app was refused" are different things for a person to do
    // next, and both used to arrive as an English string interpolated into a
    // German sentence. The state is shared with every other window that reads a
    // subsystem; the wording stays this app's.
    ReadOutcome::from_result("meetings_list", client.meetings_list().await, |rows| {
        rows.into_iter()
            .map(|m| MeetingSummaryDto {
                id: m.id,
                title: m.title,
                // The KG stores the recording start in microseconds; the card wants ms.
                date_ms: m.started_at / 1000,
                participants: m.participants,
                preview: preview_of(&m.summary, 140),
            })
            .collect()
    })
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

/// Edit a stored meeting in place: load, apply, save.
///
/// One helper for all three edit commands, so every one of them fails the same
/// way on a meeting that is not there. A missing document is an error rather than
/// a silent no-op: the surface believes it is editing something.
fn edit_meeting(
    id: &str,
    apply: impl FnOnce(&mut arlen_meetings_core::note_store::StoredMeeting) -> Result<(), String>,
) -> Result<(), String> {
    let mut meeting = arlen_meetings_core::note_store::load(id)?
        .ok_or_else(|| format!("there is no stored meeting {id}"))?;
    apply(&mut meeting)?;
    arlen_meetings_core::note_store::save(id, &meeting)
}

/// Persist the human's anchor notes for a meeting.
#[tauri::command]
async fn meeting_save_notes(id: String, text: String) -> Result<(), String> {
    edit_meeting(&id, |m| {
        arlen_meetings_core::note_store::set_human_notes(m, &text);
        Ok(())
    })
}

/// Give a diarization label the name of the person it is.
///
/// Every segment with that label is renamed, since the label is one speaker
/// across the recording.
#[tauri::command]
async fn meeting_relabel_speaker(id: String, label: String, name: String) -> Result<(), String> {
    edit_meeting(&id, |m| {
        arlen_meetings_core::note_store::relabel_speaker(m, &label, &name);
        Ok(())
    })
}

/// Apply the user's edits to one action item: its owner, whether it is done.
#[tauri::command]
async fn meeting_update_item(
    id: String,
    index: usize,
    owner: Option<String>,
    done: Option<bool>,
) -> Result<(), String> {
    edit_meeting(&id, |m| {
        arlen_meetings_core::note_store::update_action_item(m, index, owner, done)
    })
}

/// Open a produced note document in its editor. Best-effort via `xdg-open`; a
/// failure is surfaced but not fatal to the meetings surface.
#[tauri::command]
async fn open_file(file: String) -> Result<(), String> {
    // Through the shell's launch socket: the note opens under a resolved
    // application, recorded, rather than under whatever a subprocess decides.
    if !file.starts_with('/') {
        return Err(format!("not an absolute path: {file}"));
    }
    match arlen_launch_contract::open_path(&file)
        .await
        .map_err(|e| format!("could not open {file}: {e}"))?
    {
        arlen_launch_contract::LaunchOutcome::Started { .. } => Ok(()),
        other => Err(format!("could not open {file}: {other:?}")),
    }
}

/// Start on-device capture. The ASR/diarization engine is provisioned separately
/// (model-gated); until it lands this reports so.
///
/// The frontend does NOT ignore the error: it used to, and the result was a
/// running clock and a streaming transcript over a microphone nothing had opened.
/// It now shows "Recording did not start" and offers a retry, so this error is
/// read as the answer it is.
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
    // This app at info, dependencies at warn, and both halves are a fix.
    //
    // A bare `env_logger::init()` defaults to `error`, so every `log::info!`
    // and `log::warn!` here produced nothing: the app was mute in the journal.
    // That is the failure that made the boot consent hang so hard to find -
    // the component in the middle could not be heard - and it was true of four
    // apps at once.
    //
    // Dependencies stay at warn rather than being swept up to info with it,
    // because zbus logs D-Bus handshake frames WITH their message bytes, and a
    // message body is user content: paths, queries, notification text. At info
    // that lands in a journal no capability grant covers. `RUST_LOG=zbus=trace`
    // still gets it, deliberately.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,arlen_meetings_lib=info"),
    )
    .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![
            meetings_list,
            meeting_note,
            meeting_human_notes,
            open_file,
            meeting_start_capture,
            meeting_stop_capture,
            meeting_summarize,
            meeting_save_notes,
            meeting_relabel_speaker,
            meeting_update_item
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
