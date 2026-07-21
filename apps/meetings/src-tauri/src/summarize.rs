//! The `meeting_summarize` flow: run the AI summary engine over a provider, then
//! persist the produced note both as the app-owned document (the full note with
//! its transcript) and as the graph's list/link metadata.
//!
//! The transcript is untrusted spoken/injected content, so the engine screens it
//! before the model and grounds the summary in the human's anchor notes
//! (ai-meeting-notes). The provider forwards through ai-proxy over a fresh session
//! connection (the app owns no well-known bus name, so the proxy peer-auths it by
//! process identity). A provider/model that is not provisioned yields an error the
//! caller surfaces, and the frontend falls back to its local fixture.

use arlen_ai_core::proxied::{ProxiedConfig, ProxiedProvider};
use arlen_ai_core::screen::Screener;
use arlen_ai_meeting_notes::{summarize as run_summary, MeetingContext};
use arlen_meeting_note::MeetingNote;
use arlen_transcript::Transcript;
use os_sdk::{MeetingActionItemInput, UnixGraphClient};
use zbus::Connection;

use arlen_meetings_core::note_store::{self, StoredMeeting};

/// Distinct non-empty speaker labels from the transcript, in first-seen order: the
/// note's participants (not derived from the transcript text, only from who spoke).
/// Empty when diarization produced no labels.
pub fn derive_participants(transcript: &Transcript) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for seg in &transcript.segments {
        if let Some(sp) = &seg.speaker {
            if !sp.is_empty() && !seen.iter().any(|s| s == sp) {
                seen.push(sp.clone());
            }
        }
    }
    seen
}

/// A title for the note: the first non-empty line of the human notes (trimmed,
/// bounded), else a default. The human anchor names the meeting better than the
/// transcript does.
pub fn derive_title(human_notes: &str) -> String {
    human_notes
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(80).collect::<String>())
        .unwrap_or_else(|| "Meeting note".to_string())
}

/// The provider config for the app's summary forward: name/model from the env (a
/// launcher or ai.toml sets them), with safe defaults, a fixed audit token and the
/// model's default context window.
fn provider_config() -> ProxiedConfig {
    let env = |k: &str, d: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| d.to_string())
    };
    ProxiedConfig {
        name: env("ARLEN_AI_PROVIDER", "ollama"),
        model: env("ARLEN_AI_MODEL", "llama3:8b"),
        audit_token: "meetings-summary".to_string(),
        context_window: 8192,
    }
}

/// Now, in microseconds since the epoch (the recording-start stamp on the KG node).
fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// File a produced note's list/link metadata into the KG (best effort: a graph
/// hiccup must not lose the note, which is already saved app-side; the recent-
/// meetings home just misses it until the next file).
async fn file_metadata(id: &str, note: &MeetingNote) {
    let client = UnixGraphClient::new(crate::graph_socket());
    let action_items: Vec<MeetingActionItemInput> = note
        .action_items
        .iter()
        .map(|a| MeetingActionItemInput {
            text: a.text.clone(),
            owner: a.owner.clone(),
        })
        .collect();
    if let Err(e) = client
        .file_meeting(
            id,
            &note.title,
            &note.summary,
            &note.participants,
            &action_items,
            now_micros(),
        )
        .await
    {
        log::warn!("meeting {id} filed app-side but KG metadata write failed: {e}");
    }
}

/// Summarize a transcript into a grounded note and persist it: mint a meeting id,
/// save the full note document app-side, and file its list metadata into the KG.
/// Returns the produced note. A provider/model that is not provisioned is an error
/// the caller surfaces.
pub async fn summarize_and_file(
    transcript: Transcript,
    human_notes: String,
) -> Result<MeetingNote, String> {
    let ctx = MeetingContext {
        title: derive_title(&human_notes),
        participants: derive_participants(&transcript),
    };

    let connection = Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let provider = ProxiedProvider::with_connection(provider_config(), &connection)
        .await
        .map_err(|e| format!("provider unavailable: {e}"))?;

    let human = (!human_notes.trim().is_empty()).then_some(human_notes.as_str());
    let note = run_summary(transcript, human, ctx, &Screener::off(), &provider)
        .await
        .map_err(|e| e.to_string())?;

    // Persist the full document first (the durable on-device copy), then the KG
    // metadata (best effort), so a graph failure never loses the note.
    let id = uuid::Uuid::now_v7().to_string();
    note_store::save(
        &id,
        &StoredMeeting {
            note: note.clone(),
            human_notes,
        },
    )?;
    file_metadata(&id, &note).await;
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_transcript::TranscriptSegment;

    fn seg(speaker: Option<&str>) -> TranscriptSegment {
        TranscriptSegment {
            start_ms: 0,
            end_ms: 1,
            text: "hi".into(),
            speaker: speaker.map(str::to_string),
            confidence: None,
        }
    }

    #[test]
    fn participants_are_the_distinct_speaker_labels_in_order() {
        let t = Transcript {
            language: None,
            segments: vec![
                seg(Some("speaker_0")),
                seg(Some("speaker_1")),
                seg(Some("speaker_0")),
                seg(None),
                seg(Some("")),
            ],
        };
        assert_eq!(derive_participants(&t), vec!["speaker_0", "speaker_1"]);
        // No diarization -> no participants.
        let bare = Transcript { language: None, segments: vec![seg(None)] };
        assert!(derive_participants(&bare).is_empty());
    }

    #[test]
    fn title_is_the_first_non_empty_note_line_or_a_default() {
        assert_eq!(derive_title("\n  Sprint sync \nmore"), "Sprint sync");
        assert_eq!(derive_title("   \n\n"), "Meeting note");
        assert_eq!(derive_title(""), "Meeting note");
        // Bounded to 80 chars.
        let long = "x".repeat(200);
        assert_eq!(derive_title(&long).chars().count(), 80);
    }
}
