//! The meeting-notes engine: a transcript in, a [`MeetingNote`] out.
//!
//! The transcript is UNTRUSTED spoken or injected content, so the engine screens it before
//! any model reads it (the injection-isolation edge), tags it as external content in the
//! prompt so the model treats it as data, anchors the summary on the human's own notes when
//! present (the Granola pattern), and validates the model's structured reply before building
//! the note. Every failure is fail-closed: a blocked transcript, an unreachable provider or a
//! malformed reply yields an error, never a half-formed note.
//!
//! The real provider and the German-fine-tuned ASR that feeds it are provisioned separately;
//! the engine is written against the `AIProvider` seam so it is exercised here with a mock.

use arlen_ai_core::provider::{AIProvider, CompletionRequest, ProviderError};
use arlen_ai_core::screen::{Screener, Verdict};
use arlen_ai_core::tagging::{Block, Origin, TaggedPrompt};
use arlen_meeting_note::{ActionItem, MeetingNote};
use arlen_transcript::Transcript;
use serde::Deserialize;

/// Why the engine could not produce a note.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The injection screen blocked the transcript, so it was never sent to the model.
    #[error("the transcript was blocked by the injection screen")]
    Screened,
    /// The provider call failed.
    #[error("provider: {0}")]
    Provider(#[from] ProviderError),
    /// The model reply was not the expected note JSON.
    #[error("the model reply was not the expected note json")]
    Malformed,
    /// The model produced an empty summary; a note with no summary is not useful.
    #[error("the model produced an empty summary")]
    EmptySummary,
}

/// The meeting facts that name the note but are not derived from the transcript.
pub struct MeetingContext {
    /// The note title (e.g. the meeting name).
    pub title: String,
    /// The participant display names.
    pub participants: Vec<String>,
}

/// The structured reply the model is asked to return.
#[derive(Deserialize)]
struct NoteDraft {
    summary: String,
    #[serde(default)]
    action_items: Vec<DraftItem>,
}

#[derive(Deserialize)]
struct DraftItem {
    text: String,
    #[serde(default)]
    owner: Option<String>,
}

/// The instruction channel: static text, never the untrusted transcript.
const INSTRUCTIONS: &str = "You are summarizing a meeting from its transcript. \
Reply with ONLY a JSON object of the form {\"summary\": string, \"action_items\": \
[{\"text\": string, \"owner\": string or null}]}. The summary is concise prose covering \
what was discussed and decided. Ground everything strictly in the transcript and the human \
notes; do not invent participants, decisions, or tasks that are not present. Emit no text \
outside the JSON object.";

/// Screen the transcript, prompt the provider for a grounded summary, and validate the reply
/// into a [`MeetingNote`]. Fail-closed at every step.
pub async fn summarize(
    transcript: Transcript,
    human_notes: Option<&str>,
    ctx: MeetingContext,
    screener: &Screener,
    provider: &dyn AIProvider,
) -> Result<MeetingNote, EngineError> {
    let full = transcript.full_text();

    // The transcript is untrusted; screen it before the model sees it.
    if screener.screen(&full).await == Verdict::Block {
        return Err(EngineError::Screened);
    }

    let prompt = build_prompt(&full, human_notes);
    let reply = provider
        .complete(CompletionRequest { prompt, extras: serde_json::Value::Null })
        .await?;

    build_note(&reply.text, ctx, transcript)
}

/// Compose the prompt: the transcript tagged as external content, the human notes (when
/// present) as user input, wrapped in the data-only preamble.
fn build_prompt(transcript_text: &str, human_notes: Option<&str>) -> String {
    let mut blocks = vec![Block { origin: Origin::ExternalContent, content: transcript_text }];
    if let Some(notes) = human_notes {
        blocks.push(Block { origin: Origin::UserInput, content: notes });
    }
    let tagged = TaggedPrompt::new(&blocks);
    format!("{INSTRUCTIONS}\n\n{}\n\n{}", tagged.preamble(), tagged.rendered())
}

/// Parse and validate a model reply into a note, embedding the transcript. Pure and
/// synchronous: the trust-bearing validation is testable without a provider.
fn build_note(
    reply: &str,
    ctx: MeetingContext,
    transcript: Transcript,
) -> Result<MeetingNote, EngineError> {
    let draft = parse_note_json(reply).ok_or(EngineError::Malformed)?;
    let summary = draft.summary.trim();
    if summary.is_empty() {
        return Err(EngineError::EmptySummary);
    }
    let action_items = draft
        .action_items
        .into_iter()
        .filter(|i| !i.text.trim().is_empty())
        .map(|i| {
            let text = i.text.trim().to_string();
            // Ground the item to its transcript span deterministically (below), for
            // the click-to-transcript surface. Content-matched, not model-attributed.
            let source_segment = ground_to_segment(&text, &transcript);
            ActionItem {
                text,
                owner: i.owner.map(|o| o.trim().to_string()).filter(|o| !o.is_empty()),
                source_segment,
            }
        })
        .collect();
    Ok(MeetingNote {
        title: ctx.title,
        participants: ctx.participants,
        summary: summary.to_string(),
        action_items,
        transcript,
    })
}

/// The lowercased content words of a string (alphanumeric runs over two chars, minus a
/// small stopword set), for the deterministic action-item grounding. Keying on meaningful
/// terms keeps a match from riding on filler words.
fn significant_words(s: &str) -> std::collections::HashSet<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "are", "was", "will", "with", "that", "this", "you", "your",
        "our", "let", "get", "can", "should", "need", "into", "from", "have", "has",
    ];
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// Deterministically ground an action item to the transcript segment it was most plainly
/// derived from, or `None`. Content-matched, NOT model-attributed: the segment whose words
/// most cover the item's significant words wins, and only on strong, unambiguous coverage -
/// a weak or tied match is left unlinked rather than citing a segment the item did not come
/// from. A hijacked transcript line thus cannot earn a citation it does not literally match,
/// and the surface never shows a fabricated "from here" link.
fn ground_to_segment(text: &str, transcript: &Transcript) -> Option<usize> {
    let item_words = significant_words(text);
    if item_words.is_empty() {
        return None;
    }
    let mut scores: Vec<(usize, f32)> = transcript
        .segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let seg_words = significant_words(&seg.text);
            let covered = item_words.iter().filter(|w| seg_words.contains(*w)).count();
            (i, covered as f32 / item_words.len() as f32)
        })
        .collect();
    // Highest coverage first, earliest segment as a stable tiebreak.
    scores.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0))
    });
    let (idx, best) = *scores.first()?;
    let second = scores.get(1).map(|s| s.1).unwrap_or(0.0);
    // Fail-closed: strong coverage AND a clear margin over the runner-up, else unlinked.
    if best >= 0.6 && best - second >= 0.2 {
        Some(idx)
    } else {
        None
    }
}

/// Extract the note JSON from a reply, tolerating prose around it. A local model often adds
/// an aside despite the instructions, so we find the first object and let serde read exactly
/// one value from there, ignoring any trailing text. serde's streaming reader is string-aware,
/// so a brace inside a string value (or in trailing prose) does not end the object early or
/// extend it, which a naive first-`{`-to-last-`}` span would get wrong.
fn parse_note_json(reply: &str) -> Option<NoteDraft> {
    let start = reply.find('{')?;
    serde_json::Deserializer::from_str(&reply[start..])
        .into_iter::<NoteDraft>()
        .next()?
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_core::provider::{CompletionResponse, ProviderAudit};
    use arlen_transcript::TranscriptSegment;

    fn ctx() -> MeetingContext {
        MeetingContext { title: "Sync".into(), participants: vec!["Ada".into()] }
    }

    fn transcript() -> Transcript {
        Transcript {
            language: None,
            segments: vec![TranscriptSegment {
                start_ms: 0,
                end_ms: 1000,
                text: "we shipped the parser".into(),
                speaker: None,
                confidence: None,
            }],
        }
    }

    struct MockProvider {
        reply: String,
    }

    #[async_trait::async_trait]
    impl AIProvider for MockProvider {
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                text: self.reply.clone(),
                audit: ProviderAudit {
                    provider_name: "mock".into(),
                    model: "mock".into(),
                    input_tokens: None,
                    output_tokens: None,
                },
            })
        }
        async fn available(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn build_note_parses_and_validates() {
        let note = build_note(
            r#"{"summary": "we shipped", "action_items": [{"text": "write docs", "owner": "Ada"}, {"text": "  ", "owner": null}]}"#,
            ctx(),
            transcript(),
        )
        .unwrap();
        assert_eq!(note.summary, "we shipped");
        // the blank action item is dropped
        assert_eq!(note.action_items.len(), 1);
        assert_eq!(note.action_items[0].text, "write docs");
        assert_eq!(note.action_items[0].owner.as_deref(), Some("Ada"));
    }

    fn seg(text: &str) -> TranscriptSegment {
        TranscriptSegment { start_ms: 0, end_ms: 1, text: text.into(), speaker: None, confidence: None }
    }

    #[test]
    fn action_items_ground_to_their_transcript_span_and_fail_closed_when_unclear() {
        let t = Transcript {
            language: None,
            segments: vec![
                seg("Ada will migrate the billing database this week"),
                seg("we should also refresh the marketing landing page"),
                seg("great, thanks everyone"),
            ],
        };
        // A clear derivation links to its segment; an item with no strong match is unlinked.
        let note = build_note(
            r#"{"summary": "s", "action_items": [
                {"text": "migrate the billing database", "owner": "Ada"},
                {"text": "schedule the offsite retreat", "owner": null}
            ]}"#,
            ctx(),
            t,
        )
        .unwrap();
        assert_eq!(note.action_items[0].source_segment, Some(0), "strong, unambiguous match links");
        assert_eq!(note.action_items[1].source_segment, None, "no fabricated citation for an unmatched item");
    }

    #[test]
    fn a_tie_between_segments_leaves_the_item_unlinked() {
        // Two segments cover the item's words equally: fail-closed rather than pick one.
        let t = Transcript {
            language: None,
            segments: vec![seg("update the roadmap"), seg("update the roadmap")],
        };
        let note = build_note(
            r#"{"summary": "s", "action_items": [{"text": "update the roadmap", "owner": null}]}"#,
            ctx(),
            t,
        )
        .unwrap();
        assert_eq!(note.action_items[0].source_segment, None, "an ambiguous tie is not cited");
    }

    #[test]
    fn build_note_tolerates_prose_around_the_json() {
        let note = build_note(
            "Sure, here is the note:\n{\"summary\": \"done\", \"action_items\": []}\nHope that helps!",
            ctx(),
            transcript(),
        )
        .unwrap();
        assert_eq!(note.summary, "done");
        assert!(note.action_items.is_empty());
    }

    #[test]
    fn build_note_ignores_trailing_prose_that_contains_a_brace() {
        // a non-compliant model adds an aside with its own brace; a first-{-to-last-} span
        // would over-extend and fail, so the streaming parse must stop at the first value.
        let note = build_note(
            r#"{"summary": "shipped", "action_items": []} (kept it brief {as asked})"#,
            ctx(),
            transcript(),
        )
        .unwrap();
        assert_eq!(note.summary, "shipped");
    }

    #[test]
    fn build_note_rejects_a_non_json_reply() {
        let err = build_note("I could not summarize this.", ctx(), transcript()).unwrap_err();
        assert!(matches!(err, EngineError::Malformed));
    }

    #[test]
    fn build_note_rejects_an_empty_summary() {
        let err = build_note(
            r#"{"summary": "   ", "action_items": []}"#,
            ctx(),
            transcript(),
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::EmptySummary));
    }

    #[tokio::test]
    async fn summarize_screens_then_builds_the_note() {
        let provider = MockProvider {
            reply: r#"{"summary": "we shipped the parser", "action_items": []}"#.into(),
        };
        let note = summarize(transcript(), None, ctx(), &Screener::off(), &provider)
            .await
            .unwrap();
        assert_eq!(note.summary, "we shipped the parser");
        // the transcript is embedded verbatim for the note document
        assert_eq!(note.transcript.full_text(), "we shipped the parser");
    }

    struct PanicProvider;

    #[async_trait::async_trait]
    impl AIProvider for PanicProvider {
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            panic!("the model must never be called when the transcript is screened");
        }
        async fn available(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "panic"
        }
    }

    #[tokio::test]
    async fn a_screened_transcript_never_reaches_the_model() {
        use arlen_ai_core::screen::ScreeningMode;
        // the injection-isolation edge: a blocking screen stops the untrusted transcript
        // before it is ever sent to the model (PanicProvider fails the test if it is called).
        let err = summarize(
            transcript(),
            None,
            ctx(),
            &Screener::new(ScreeningMode::FailClosed),
            &PanicProvider,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, EngineError::Screened));
    }

    #[test]
    fn the_prompt_tags_the_transcript_as_external_content() {
        let prompt = build_prompt("secret transcript", Some("my notes"));
        assert!(prompt.contains("EXTERNAL-CONTENT"));
        assert!(prompt.contains("USER-QUESTION"));
        assert!(prompt.contains("secret transcript"));
        assert!(prompt.contains("DATA ONLY"));
    }
}
