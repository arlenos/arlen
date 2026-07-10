//! The app-owned meeting-note document store. A produced note stays fully
//! on-device (the Otter/Granola trap we avoid): the graph holds only list/link
//! metadata, and the full note (summary + action items + the transcript it was
//! grounded in) plus the human's anchor notes are persisted here as one JSON
//! document per meeting, under `$XDG_DATA_HOME/arlen/meetings/`.
//!
//! `meeting_note` loads a past note from here (the transcript the KG does not
//! carry); the summarize-and-file flow writes it (the write side is the mechanism
//! the AI-summary slice calls once a provider is wired).

use std::path::PathBuf;

use arlen_meeting_note::MeetingNote;
use serde::{Deserialize, Serialize};

/// A persisted meeting: the produced note plus the human's anchor notes (held
/// beside the note, never folded into the model-derived `MeetingNote`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredMeeting {
    /// The full produced note (summary + action items + transcript).
    pub note: MeetingNote,
    /// The notes the human typed during the meeting, the anchor that grounds the
    /// summary and suppresses hallucination.
    #[serde(default)]
    pub human_notes: String,
}

/// The meetings document directory: `$XDG_DATA_HOME/arlen/meetings`, else
/// `$HOME/.local/share/arlen/meetings`.
fn meetings_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".local/share")
        });
    base.join("arlen/meetings")
}

/// Whether a meeting id is safe as a filename component: non-empty and only
/// `[A-Za-z0-9._-]`, with no `.`/`..` and no separator, so a caller id can never
/// escape the meetings directory. Meeting ids are app-minted (a UUID), so this is
/// a defensive floor, not an expected-rejection path.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The document path for a meeting id (already validated by the caller).
fn note_path(id: &str) -> PathBuf {
    meetings_dir().join(format!("{id}.json"))
}

/// Persist a produced note document for `id`, creating the meetings directory if
/// needed. Atomic (write a sibling temp file then rename), so a concurrent load
/// never sees a half-written document. Idempotent: a re-save overwrites in place.
///
/// The write side is the mechanism the summarize-and-file flow calls once a
/// provider is wired (that command is a stub today), so it has no production caller
/// yet; the load side is live via `meeting_note`.
#[allow(dead_code)]
pub fn save(id: &str, meeting: &StoredMeeting) -> Result<(), String> {
    if !is_safe_id(id) {
        return Err(format!("unsafe meeting id: {id}"));
    }
    let dir = meetings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create meetings dir: {e}"))?;
    let json = serde_json::to_vec_pretty(meeting).map_err(|e| format!("serialize note: {e}"))?;
    let tmp = dir.join(format!(".{id}.json.tmp"));
    std::fs::write(&tmp, &json).map_err(|e| format!("write note: {e}"))?;
    std::fs::rename(&tmp, note_path(id)).map_err(|e| format!("commit note: {e}"))?;
    Ok(())
}

/// Load a past note document by id, or `None` when there is none. A malformed
/// document is an error (a corrupt file is surfaced, not silently dropped).
pub fn load(id: &str) -> Result<Option<StoredMeeting>, String> {
    if !is_safe_id(id) {
        return Err(format!("unsafe meeting id: {id}"));
    }
    let path = note_path(id);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let stored: StoredMeeting =
                serde_json::from_slice(&bytes).map_err(|e| format!("parse note {id}: {e}"))?;
            Ok(Some(stored))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read note {id}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_transcript::{Transcript, TranscriptSegment};

    fn note() -> MeetingNote {
        MeetingNote {
            title: "Sync".into(),
            participants: vec!["Tim".into(), "Ada, the reviewer".into()],
            summary: "we shipped the parser".into(),
            action_items: vec![],
            transcript: Transcript {
                language: Some("en".into()),
                segments: vec![TranscriptSegment {
                    start_ms: 0,
                    end_ms: 1000,
                    text: "we shipped".into(),
                    speaker: Some("speaker_0".into()),
                    confidence: Some(0.9),
                }],
            },
        }
    }

    #[test]
    fn is_safe_id_rejects_traversal_and_separators() {
        assert!(is_safe_id("019abc-uuid"));
        for bad in ["", ".", "..", "a/b", "../etc", "a b", "a\0b"] {
            assert!(!is_safe_id(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn save_then_load_round_trips_the_full_note_and_human_notes() {
        // Isolate the store under a temp XDG_DATA_HOME for this test.
        let tmp = std::env::temp_dir().join(format!("arlen-meetings-test-{}", std::process::id()));
        std::env::set_var("XDG_DATA_HOME", &tmp);

        let stored = StoredMeeting { note: note(), human_notes: "my anchor".into() };
        save("m-1", &stored).unwrap();
        let got = load("m-1").unwrap().expect("saved");
        assert_eq!(got, stored);
        // The transcript survives (the KG does not carry it).
        assert_eq!(got.note.transcript.segments[0].text, "we shipped");
        assert_eq!(got.human_notes, "my anchor");

        // An unknown id is None, not an error.
        assert!(load("nope").unwrap().is_none());

        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
