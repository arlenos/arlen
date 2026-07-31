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
/// The write side is called by the summarize-and-file flow (`summarize.rs`); the
/// load side is called by `meeting_note`.
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
mod edit_tests {
    use super::*;
    use arlen_meeting_note::{ActionItem, MeetingNote};
    use arlen_transcript::{Transcript, TranscriptSegment};

    fn segment(text: &str, speaker: Option<&str>) -> TranscriptSegment {
        TranscriptSegment {
            start_ms: 0,
            end_ms: 1,
            text: text.into(),
            speaker: speaker.map(|s| s.to_string()),
            confidence: None,
        }
    }

    fn meeting() -> StoredMeeting {
        StoredMeeting {
            note: MeetingNote {
                title: "Standup".into(),
                participants: vec![],
                summary: String::new(),
                summary_claims: vec![],
                action_items: vec![ActionItem {
                    text: "Write it up".into(),
                    owner: None,
                    source_segment: None,
                    done: false,
                }],
                transcript: Transcript {
                    language: None,
                    segments: vec![
                        segment("morning", Some("speaker_0")),
                        segment("hello", Some("speaker_1")),
                        segment("and again", Some("speaker_0")),
                    ],
                },
            },
            human_notes: String::new(),
        }
    }

    /// One label is one speaker across the whole recording, so renaming has to
    /// catch every segment or it splits a person in two.
    #[test]
    fn a_relabel_renames_every_segment_of_that_speaker() {
        let mut m = meeting();
        assert_eq!(relabel_speaker(&mut m, "speaker_0", "Ada"), 2);
        let speakers: Vec<_> = m
            .note
            .transcript
            .segments
            .iter()
            .map(|s| s.speaker.clone().unwrap_or_default())
            .collect();
        assert_eq!(speakers, vec!["Ada", "speaker_1", "Ada"]);
    }

    /// A rename to nothing is not a rename, and clearing the diarization is not
    /// what the user asked for.
    #[test]
    fn an_empty_name_changes_nothing() {
        let mut m = meeting();
        assert_eq!(relabel_speaker(&mut m, "speaker_0", "   "), 0);
        assert_eq!(m.note.transcript.segments[0].speaker.as_deref(), Some("speaker_0"));
    }

    /// A label nobody carries reports zero rather than appearing to succeed.
    #[test]
    fn relabelling_an_absent_speaker_reports_nothing_changed() {
        let mut m = meeting();
        assert_eq!(relabel_speaker(&mut m, "speaker_7", "Ada"), 0);
    }

    /// Ticking a box must not restate the owner, so an omitted field stands.
    #[test]
    fn an_omitted_field_leaves_the_item_alone() {
        let mut m = meeting();
        update_action_item(&mut m, 0, Some("Ada".into()), None).unwrap();
        update_action_item(&mut m, 0, None, Some(true)).unwrap();
        assert_eq!(m.note.action_items[0].owner.as_deref(), Some("Ada"));
        assert!(m.note.action_items[0].done);
    }

    /// An owner cleared to blank is unassigned, not a person named "".
    #[test]
    fn a_blank_owner_unassigns() {
        let mut m = meeting();
        update_action_item(&mut m, 0, Some("Ada".into()), None).unwrap();
        update_action_item(&mut m, 0, Some("  ".into()), None).unwrap();
        assert_eq!(m.note.action_items[0].owner, None);
    }

    /// The list the user clicked and the document on disk have disagreed, and
    /// appearing to succeed would hide that.
    #[test]
    fn an_index_past_the_end_is_refused() {
        let mut m = meeting();
        assert!(update_action_item(&mut m, 5, None, Some(true)).is_err());
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
            summary_claims: Vec::new(),
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

/// Replace the human's anchor notes for a stored meeting.
///
/// The anchor notes are the human's own, so this overwrites rather than merges:
/// what the editor holds IS the document, and a merge would resurrect text the
/// person deleted.
pub fn set_human_notes(meeting: &mut StoredMeeting, text: &str) {
    meeting.human_notes = text.to_string();
}

/// Give a diarization label (`speaker_0`) the name a human recognised.
///
/// Rewrites every segment carrying that label, because the label is one speaker
/// across the whole recording and renaming half of them would split one person in
/// two. Returns how many segments changed, so a relabel that matched nothing is
/// distinguishable from one that matched everything.
///
/// An empty name is a no-op: clearing a speaker back to nothing is what deleting
/// the diarization would mean, and this is a rename.
pub fn relabel_speaker(meeting: &mut StoredMeeting, label: &str, name: &str) -> usize {
    if name.trim().is_empty() {
        return 0;
    }
    let mut changed = 0;
    for segment in &mut meeting.note.transcript.segments {
        if segment.speaker.as_deref() == Some(label) {
            segment.speaker = Some(name.to_string());
            changed += 1;
        }
    }
    changed
}

/// Apply the user's edits to one action item: who owns it, whether it is done.
///
/// `None` for either field leaves it as it stands, so the caller can tick a box
/// without restating the owner. An index past the end is refused rather than
/// silently ignored: the list the user clicked and the document on disk have
/// disagreed, and appearing to succeed would hide that.
pub fn update_action_item(
    meeting: &mut StoredMeeting,
    index: usize,
    owner: Option<String>,
    done: Option<bool>,
) -> Result<(), String> {
    let item = meeting
        .note
        .action_items
        .get_mut(index)
        .ok_or_else(|| format!("this meeting has no action item {index}"))?;
    if let Some(owner) = owner {
        // An owner cleared to nothing is unassigned, not a person named "".
        item.owner = Some(owner).filter(|o| !o.trim().is_empty());
    }
    if let Some(done) = done {
        item.done = done;
    }
    Ok(())
}
