//! The transcription contract: what an ASR component hands the meeting-notes and
//! shell-voice engines, plus the post-processing every consumer shares.
//!
//! An ASR pass (faster-whisper) emits many short, time-stamped [`TranscriptSegment`]s;
//! a diarization pass labels each with a speaker. The engines then want the full text
//! (to feed a summarizer), a lookup from a playback timestamp to the segment under it
//! (the click-to-transcript affordance), and adjacent same-speaker segments folded into
//! utterances. That shared shape lives here so the ASR producer, the meeting-notes
//! engine and the voice-dictation path agree on one model.
//!
//! Transcript text is UNTRUSTED input (it carries whatever was spoken or injected). This
//! crate only models and reshapes it; screening it before a model reads it is the
//! consumer's job (the injection-isolation edge), never assumed here.

use serde::{Deserialize, Serialize};

/// One recognized span of speech: a half-open time range `[start_ms, end_ms)` from the
/// start of the recording, the recognized text, an optional diarization speaker label,
/// and the ASR's optional confidence in the span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Start offset from the recording start, in milliseconds (inclusive).
    pub start_ms: u64,
    /// End offset from the recording start, in milliseconds (exclusive). Never less
    /// than `start_ms`.
    pub end_ms: u64,
    /// The recognized text for this span.
    pub text: String,
    /// The diarization speaker label (e.g. `"speaker_0"`), or `None` when diarization
    /// did not run or could not attribute the span.
    #[serde(default)]
    pub speaker: Option<String>,
    /// The ASR's confidence in this span, `0.0..=1.0`, or `None` when the backend does
    /// not report one.
    #[serde(default)]
    pub confidence: Option<f32>,
}

/// A full transcription: the ordered segments plus the detected language.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// The detected language as a BCP-47-ish tag (e.g. `"de"`, `"en"`), or `None` when
    /// the backend did not report one.
    #[serde(default)]
    pub language: Option<String>,
    /// The segments in recording order.
    pub segments: Vec<TranscriptSegment>,
}

impl Transcript {
    /// The concatenated segment texts, one space between segments, for a summarizer or
    /// an entity pass. Empty when there are no segments.
    pub fn full_text(&self) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            let piece = seg.text.trim();
            if piece.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(piece);
        }
        out
    }

    /// The recording duration this transcript spans, in milliseconds: the largest
    /// segment `end_ms`. Zero for an empty transcript. Robust to out-of-order segments.
    pub fn duration_ms(&self) -> u64 {
        self.segments.iter().map(|s| s.end_ms).max().unwrap_or(0)
    }

    /// The segment whose `[start_ms, end_ms)` range contains `ms` (the click-to-transcript
    /// lookup: given a playback position, which line is being spoken). Returns the first
    /// match in recording order, or `None` when no segment covers `ms`.
    pub fn segment_at_ms(&self, ms: u64) -> Option<&TranscriptSegment> {
        self.segments
            .iter()
            .find(|s| ms >= s.start_ms && ms < s.end_ms)
    }

    /// Whether the segments are non-overlapping and in non-decreasing start order, and
    /// each range is well-formed (`start_ms <= end_ms`). A producer should emit a
    /// well-formed transcript; a consumer can assert it before relying on the ordering
    /// invariants the lookups above use.
    pub fn is_well_formed(&self) -> bool {
        let mut prev_end = 0u64;
        let mut prev_start = 0u64;
        for (i, seg) in self.segments.iter().enumerate() {
            if seg.start_ms > seg.end_ms {
                return false;
            }
            if i > 0 && (seg.start_ms < prev_start || seg.start_ms < prev_end) {
                return false;
            }
            prev_start = seg.start_ms;
            prev_end = seg.end_ms;
        }
        true
    }

    /// Fold runs of adjacent segments that share a speaker into one utterance each: the
    /// text is joined with a space, the range is widened to cover the run, and the
    /// confidence becomes the run minimum (the weakest link, the conservative signal a
    /// low-hallucination summary wants). Segments with no speaker label are never merged
    /// (without diarization there is nothing to group by), so a transcript that never ran
    /// diarization is returned unchanged. The result stays in recording order.
    pub fn merge_adjacent_same_speaker(&self) -> Transcript {
        let mut merged: Vec<TranscriptSegment> = Vec::with_capacity(self.segments.len());
        for seg in &self.segments {
            if let Some(last) = merged.last_mut() {
                let same_named_speaker =
                    seg.speaker.is_some() && last.speaker == seg.speaker;
                if same_named_speaker {
                    if !seg.text.trim().is_empty() {
                        if !last.text.is_empty() {
                            last.text.push(' ');
                        }
                        last.text.push_str(seg.text.trim());
                    }
                    last.end_ms = last.end_ms.max(seg.end_ms);
                    last.confidence = min_confidence(last.confidence, seg.confidence);
                    continue;
                }
            }
            merged.push(TranscriptSegment {
                text: seg.text.trim().to_string(),
                ..seg.clone()
            });
        }
        Transcript {
            language: self.language.clone(),
            segments: merged,
        }
    }
}

/// The minimum of two optional confidences, treating an absent value as "no floor" so a
/// merge does not invent a confidence where none was reported.
fn min_confidence(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: u64, end: u64, text: &str, speaker: Option<&str>) -> TranscriptSegment {
        TranscriptSegment {
            start_ms: start,
            end_ms: end,
            text: text.to_string(),
            speaker: speaker.map(String::from),
            confidence: None,
        }
    }

    #[test]
    fn full_text_joins_and_skips_blank() {
        let t = Transcript {
            language: Some("en".into()),
            segments: vec![
                seg(0, 500, "hello", None),
                seg(500, 600, "   ", None),
                seg(600, 1000, "world", None),
            ],
        };
        assert_eq!(t.full_text(), "hello world");
    }

    #[test]
    fn duration_is_the_max_end() {
        let t = Transcript {
            language: None,
            segments: vec![seg(0, 500, "a", None), seg(500, 1200, "b", None)],
        };
        assert_eq!(t.duration_ms(), 1200);
        assert_eq!(Transcript::default().duration_ms(), 0);
    }

    #[test]
    fn segment_lookup_is_half_open() {
        let t = Transcript {
            language: None,
            segments: vec![seg(0, 500, "a", None), seg(500, 1000, "b", None)],
        };
        assert_eq!(t.segment_at_ms(0).unwrap().text, "a");
        assert_eq!(t.segment_at_ms(499).unwrap().text, "a");
        // 500 is the exclusive end of the first segment and the inclusive start of the next.
        assert_eq!(t.segment_at_ms(500).unwrap().text, "b");
        assert!(t.segment_at_ms(1000).is_none());
    }

    #[test]
    fn well_formed_rejects_overlap_and_reversed_range() {
        let ok = Transcript {
            language: None,
            segments: vec![seg(0, 500, "a", None), seg(500, 900, "b", None)],
        };
        assert!(ok.is_well_formed());
        let overlap = Transcript {
            language: None,
            segments: vec![seg(0, 600, "a", None), seg(500, 900, "b", None)],
        };
        assert!(!overlap.is_well_formed());
        let reversed = Transcript {
            language: None,
            segments: vec![seg(600, 500, "a", None)],
        };
        assert!(!reversed.is_well_formed());
    }

    #[test]
    fn merge_folds_same_speaker_runs_only() {
        let t = Transcript {
            language: Some("de".into()),
            segments: vec![
                TranscriptSegment {
                    confidence: Some(0.9),
                    ..seg(0, 400, "guten", Some("speaker_0"))
                },
                TranscriptSegment {
                    confidence: Some(0.6),
                    ..seg(400, 800, "morgen", Some("speaker_0"))
                },
                seg(800, 1200, "hallo", Some("speaker_1")),
            ],
        };
        let m = t.merge_adjacent_same_speaker();
        assert_eq!(m.segments.len(), 2);
        assert_eq!(m.segments[0].text, "guten morgen");
        assert_eq!(m.segments[0].start_ms, 0);
        assert_eq!(m.segments[0].end_ms, 800);
        // the run confidence is the weakest link
        assert_eq!(m.segments[0].confidence, Some(0.6));
        assert_eq!(m.segments[1].text, "hallo");
    }

    #[test]
    fn merge_leaves_unlabelled_segments_alone() {
        let t = Transcript {
            language: None,
            segments: vec![seg(0, 400, "one", None), seg(400, 800, "two", None)],
        };
        let m = t.merge_adjacent_same_speaker();
        // no diarization: nothing to group by, so both segments survive
        assert_eq!(m.segments.len(), 2);
    }

    #[test]
    fn round_trips_through_json() {
        let t = Transcript {
            language: Some("en".into()),
            segments: vec![TranscriptSegment {
                confidence: Some(0.8),
                ..seg(0, 500, "hi", Some("speaker_0"))
            }],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
