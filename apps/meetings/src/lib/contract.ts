/// TypeScript mirrors of the Rust wire contracts the coder built, so the surface is
/// typed against the real shapes: `contracts/transcript` + `contracts/meeting-note`.
/// Field names match the Rust structs (serde default), so a fixture here is the same
/// shape the daemon will send once the ASR + engine + KG-filing seams land.

/// One recognized span of speech (`contracts/transcript::TranscriptSegment`).
export interface TranscriptSegment {
  /// Start offset from the recording start, milliseconds (inclusive).
  start_ms: number;
  /// End offset, milliseconds (exclusive).
  end_ms: number;
  /// The recognized text.
  text: string;
  /// Diarization speaker label (e.g. "speaker_0"), when available.
  speaker?: string;
  /// ASR confidence in [0, 1], when available.
  confidence?: number;
}

/// A whole transcript (`contracts/transcript::Transcript`).
export interface Transcript {
  /// BCP-47 language tag (e.g. "en", "de").
  language?: string;
  /// Segments in recording order.
  segments: TranscriptSegment[];
}

/// One extracted action item (`contracts/meeting-note::ActionItem`).
export interface ActionItem {
  text: string;
  /// The person it was assigned to, when the model identified one.
  owner?: string;
}

/// The produced meeting note (`contracts/meeting-note::MeetingNote`). The summary +
/// action items are AI-generated from the transcript, grounded by the human's notes.
export interface MeetingNote {
  title: string;
  participants: string[];
  summary: string;
  action_items: ActionItem[];
  transcript: Transcript;
}
