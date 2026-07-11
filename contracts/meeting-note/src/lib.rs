//! The meeting note document: the structured artifact the meeting-notes engine
//! produces from a transcript (plus any human notes), rendered to the markdown the
//! text-editor opens as a KG citizen.
//!
//! The embedded transcript, and the summary and action items derived from it, all trace
//! back to UNTRUSTED spoken or injected content. The markdown rendering therefore
//! neutralizes structural markdown at the start of every untrusted-derived line, so a
//! transcript line like `- [ ] wire the money` or `## Decisions` cannot forge a checklist
//! item or a heading in the rendered note (the injection-isolation edge, applied at the
//! document boundary). Screening the content before a model summarizes it is the engine's
//! separate, upstream responsibility.

use arlen_transcript::Transcript;
use serde::{Deserialize, Serialize};

/// One extracted action item: the task text and, when the extractor attributed it, an
/// owner. Kept deliberately small; richer fields (due date, linked entity) land with the
/// extractor that can populate them without guessing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    /// The task text.
    pub text: String,
    /// The person the item was assigned to, when the extractor attributed one. Omitted from
    /// the wire when absent, so it reads as an optional (`owner?`) to a TS consumer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The transcript segment index this item was plainly derived from, for the
    /// click-to-transcript surface (index into [`MeetingNote::transcript`] segments).
    /// Content-matched deterministically and set only on a strong, unambiguous match,
    /// so it is `None` rather than a fabricated citation when the grounding is unclear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_segment: Option<usize>,
}

/// One sentence of the prose summary paired with the transcript segment it was plainly
/// derived from, for the click-to-transcript surface (the Granola grounded-summary
/// pattern). `source_segment` is content-matched deterministically and set only on a
/// strong, unambiguous match, so a synthesized/paraphrased claim that matches no segment
/// stays `None` rather than citing one it did not come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryClaim {
    /// The summary sentence text.
    pub text: String,
    /// The transcript segment index this claim grounds to, or `None` when unclear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_segment: Option<usize>,
}

/// A meeting note: the human-facing title, the participants, the summary, the extracted
/// action items and the transcript it was built from. The engine produces this; the
/// text-editor renders [`MeetingNote::to_markdown`] as an editable document and the
/// Knowledge app links it to the meeting's people and project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingNote {
    /// The note title (e.g. the meeting name).
    pub title: String,
    /// The participant display names, in the order to list them.
    #[serde(default)]
    pub participants: Vec<String>,
    /// The prose summary (the rendered/stored form).
    pub summary: String,
    /// The summary split into sentence-claims, each grounded to its transcript span for
    /// the click-to-transcript surface. Derived from `summary`; omitted from the wire when
    /// empty. The rendered document still uses `summary`; this is the interactive overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary_claims: Vec<SummaryClaim>,
    /// The extracted action items.
    #[serde(default)]
    pub action_items: Vec<ActionItem>,
    /// The transcript the note was built from.
    pub transcript: Transcript,
}

impl MeetingNote {
    /// Render the note as a markdown document. Untrusted-derived text (the summary, the
    /// action items and the transcript) has structural markdown neutralized at each line
    /// start so embedded content cannot forge headings, list items or quotes.
    pub fn to_markdown(&self) -> String {
        let mut md = format!("# {}\n", escape_md_block(&self.title));
        if !self.participants.is_empty() {
            let names: Vec<String> = self.participants.iter().map(|p| escape_md_inline(p)).collect();
            md.push_str(&format!("\nParticipants: {}\n", names.join(", ")));
        }

        md.push_str("\n## Summary\n\n");
        md.push_str(&escape_md_block(&self.summary));
        md.push('\n');

        md.push_str("\n## Action items\n\n");
        if self.action_items.is_empty() {
            md.push_str("_None captured._\n");
        } else {
            for item in &self.action_items {
                md.push_str("- [ ] ");
                md.push_str(&escape_md_inline(&item.text));
                if let Some(owner) = &item.owner {
                    md.push_str(&format!(" (@{})", escape_md_inline(owner)));
                }
                md.push('\n');
            }
        }

        // Render the transcript as one line per utterance (adjacent same-speaker segments
        // folded together) so the document reads naturally; the untouched `transcript` field
        // keeps the raw segments for the interactive click-to-transcript surface.
        md.push_str("\n## Transcript\n\n");
        md.push_str(&escape_md_block(
            &self.transcript.merge_adjacent_same_speaker().to_readable(),
        ));
        md.push('\n');
        md
    }
}

/// Neutralize structural markdown at the start of every line of an untrusted block: a
/// leading heading (`#`), quote (`>`) or list (`-`, `+`, `*`, `1.`) marker is backslash
/// escaped so the line renders as literal text. Line breaks are preserved. Inline emphasis
/// (`*word*`, `[text](url)`) is left as-is: it cannot forge document structure, only
/// cosmetic styling, and over-escaping it would mangle legitimate prose.
fn escape_md_block(s: &str) -> String {
    s.lines().map(escape_md_line).collect::<Vec<_>>().join("\n")
}

/// Neutralize an untrusted value used inside a single line (a name, an action item): drop
/// any line breaks so it cannot open a new structural line, then escape a leading marker.
fn escape_md_inline(s: &str) -> String {
    let one_line: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    escape_md_line(one_line.trim())
}

/// Escape a leading structural markdown marker on one line, preserving indentation.
fn escape_md_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    if starts_structural(trimmed) {
        format!("{indent}\\{trimmed}")
    } else {
        line.to_string()
    }
}

/// Whether a line (already left-trimmed) opens a markdown block structure.
fn starts_structural(t: &str) -> bool {
    t.starts_with('#')
        || t.starts_with('>')
        || t.starts_with("```")
        || t.starts_with("~~~")
        || is_bullet(t)
        || is_ordered(t)
        || is_ruler_line(t)
}

/// A setext underline (`===`, `---`) or thematic break (`---`, `***`, `___`): a line that,
/// once whitespace is removed, is a single ruler character repeated. Alone such a line
/// forges a horizontal rule, and directly under a text line it promotes that text to a
/// heading, so an injected ruler must be neutralized like any other structural marker.
fn is_ruler_line(t: &str) -> bool {
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    let mut chars = compact.chars();
    match chars.next() {
        Some(first) if matches!(first, '=' | '-' | '*' | '_') => chars.all(|c| c == first),
        _ => false,
    }
}

/// A `-`, `+` or `*` bullet (the marker alone, or followed by a space).
fn is_bullet(t: &str) -> bool {
    matches!(t, "-" | "+" | "*")
        || t.starts_with("- ")
        || t.starts_with("+ ")
        || t.starts_with("* ")
}

/// An ordered-list start: one or more digits then `.` or `)`.
fn is_ordered(t: &str) -> bool {
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let rest = &t[digits.len()..];
    rest.starts_with('.') || rest.starts_with(')')
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_transcript::TranscriptSegment;

    fn note(summary: &str, items: Vec<ActionItem>, segs: Vec<TranscriptSegment>) -> MeetingNote {
        MeetingNote {
            title: "Sprint sync".into(),
            participants: vec!["Ada".into(), "Grace".into()],
            summary: summary.into(),
            summary_claims: Vec::new(),
            action_items: items,
            transcript: Transcript { language: None, segments: segs },
        }
    }

    fn seg(text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_ms: 0,
            end_ms: 1000,
            text: text.into(),
            speaker: None,
            confidence: None,
        }
    }

    #[test]
    fn renders_the_expected_sections() {
        let n = note(
            "We shipped the parser.",
            vec![ActionItem { text: "Write the changelog".into(), owner: Some("Ada".into()), source_segment: None }],
            vec![seg("all done")],
        );
        let md = n.to_markdown();
        assert!(md.starts_with("# Sprint sync\n"));
        assert!(md.contains("Participants: Ada, Grace"));
        assert!(md.contains("## Summary\n\nWe shipped the parser."));
        assert!(md.contains("- [ ] Write the changelog (@Ada)"));
        assert!(md.contains("## Transcript\n\n[0:00] all done"));
    }

    #[test]
    fn the_transcript_renders_as_merged_utterances() {
        let segs = vec![
            TranscriptSegment {
                start_ms: 0,
                end_ms: 500,
                text: "hello".into(),
                speaker: Some("Ada".into()),
                confidence: None,
            },
            TranscriptSegment {
                start_ms: 500,
                end_ms: 1000,
                text: "there".into(),
                speaker: Some("Ada".into()),
                confidence: None,
            },
        ];
        let md = note("s", vec![], segs).to_markdown();
        // the two adjacent same-speaker segments fold into one utterance line
        assert!(md.contains("[0:00] Ada: hello there"));
    }

    #[test]
    fn empty_action_items_render_a_placeholder() {
        let md = note("nothing to do", vec![], vec![seg("ok")]).to_markdown();
        assert!(md.contains("## Action items\n\n_None captured._"));
    }

    #[test]
    fn an_injected_transcript_line_cannot_forge_structure() {
        // A speaker dictating markdown-looking text must not become a heading or a checklist
        // item in the rendered note.
        let n = note(
            "## Injected summary heading\n- [ ] steal the funds",
            vec![],
            vec![seg("please add - [ ] transfer everything")],
        );
        let md = n.to_markdown();
        // the summary's injected heading and list marker are escaped
        assert!(md.contains("\\## Injected summary heading"));
        assert!(md.contains("\\- [ ] steal the funds"));
        // the transcript line keeps its timestamp prefix, so its content stays mid-line;
        // no bare "- [ ]" appears at a line start anywhere in the transcript section
        let transcript_part = md.split("## Transcript").nth(1).unwrap();
        assert!(!transcript_part.lines().any(|l| l.trim_start().starts_with("- [ ]")));
    }

    #[test]
    fn an_injected_owner_cannot_break_the_line() {
        let n = note(
            "s",
            vec![ActionItem { text: "do it\n## Fake".into(), owner: Some("x\n- y".into()), source_segment: None }],
            vec![seg("t")],
        );
        let md = n.to_markdown();
        // the newline in the item text and owner is collapsed, so no new structural line
        assert!(md.contains("- [ ] do it ## Fake (@x - y)"));
    }

    #[test]
    fn an_injected_ruler_line_cannot_forge_a_heading_or_rule() {
        // "===" directly under a text line would promote it to a heading; "---"/"***"/"___"
        // forge horizontal rules. Each is escaped so it renders literally.
        let md = note("above\n===\n- - -\n***\n___", vec![], vec![seg("t")]).to_markdown();
        assert!(md.contains("\\==="));
        assert!(md.contains("\\- - -"));
        assert!(md.contains("\\***"));
        assert!(md.contains("\\___"));
    }

    #[test]
    fn round_trips_through_json() {
        let n = note("s", vec![ActionItem { text: "t".into(), owner: None, source_segment: None }], vec![seg("x")]);
        let json = serde_json::to_string(&n).unwrap();
        let back: MeetingNote = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
    }
}
