/// The active meeting: the human's own notes (the Granola anchor - held by this app,
/// what you jotted during capture) plus the produced MeetingNote (the engine output).
/// The verifiable merge shows your notes against the AI summary, both checkable against
/// the embedded transcript.
///
/// Mock-vs-live: fixture-backed. The ASR/diarization capture stream, the `summarize`
/// engine call, the KG-file store, and the text-editor handoff are coder seams; under
/// vite the store serves a fixture so the surface renders.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { MeetingNote, TranscriptSegment } from "$lib/contract";

/// A captured meeting: your notes (the anchor) + the produced note.
export interface Meeting {
  /// What you typed during the meeting - the black anchor that suppresses
  /// hallucination. Held by the app, never sent back inside the MeetingNote.
  humanNotes: string;
  /// The engine's output (summary + action items + transcript).
  note: MeetingNote;
  /// True while showing the fixture (no engine under vite).
  mocked: boolean;
}

export const meeting = writable<Meeting | null>(null);

const FIXTURE: Meeting = {
  mocked: true,
  humanNotes:
    "why build our own editor: KG-lens\nmeeting notes must stay on-device (Otter lawsuit)\nTim: capture is its own surface, note goes to the editor",
  note: {
    title: "Editor and meeting-notes direction",
    participants: ["You", "Tim"],
    summary:
      "The KG-lens is the reason to build a first-party editor rather than reuse gedit; a plain editor cannot show provenance and project context. Meeting capture stays fully on-device, which is the edge over cloud transcription bots. Capture lives in its own small surface and the resulting note becomes a knowledge-graph file, opened in the editor for follow-up.",
    action_items: [
      { text: "Split capture into its own Meetings surface", owner: "arlen-ui" },
      { text: "File the produced note as a knowledge-graph node", owner: "coder" },
    ],
    transcript: {
      language: "en",
      segments: [
        { start_ms: 4200, end_ms: 9800, speaker: "speaker_0", confidence: 0.95, text: "So the whole reason to build our own editor is the knowledge-graph lens." },
        { start_ms: 9800, end_ms: 15200, speaker: "speaker_0", confidence: 0.93, text: "A plain editor like gedit just cannot surface provenance or which project a file belongs to." },
        { start_ms: 15200, end_ms: 21000, speaker: "speaker_1", confidence: 0.9, text: "Right, and the meeting notes have to stay on this device. The Otter lawsuit is exactly the trap we avoid." },
        { start_ms: 21000, end_ms: 27400, speaker: "speaker_1", confidence: 0.92, text: "Let's make the capture its own small surface, and the note it produces becomes a graph file you open in the editor." },
        { start_ms: 27400, end_ms: 31900, speaker: "speaker_0", confidence: 0.94, text: "Agreed. Capture is one lifecycle, the note is a citizen of the graph after." },
      ],
    },
  },
};

/// Load the active meeting: the engine seam, with the fixture fallback under vite.
export async function loadMeeting(): Promise<void> {
  try {
    const note = await invoke<MeetingNote>("meeting_note");
    const humanNotes = await invoke<string>("meeting_human_notes");
    meeting.set({ humanNotes, note, mocked: false });
  } catch {
    meeting.set(FIXTURE);
  }
}

/// Fold adjacent same-speaker segments into utterances (mirrors the contract's
/// `merge_adjacent_same_speaker`); confidence of a run is its weakest link.
export function mergeAdjacent(segments: TranscriptSegment[]): TranscriptSegment[] {
  const out: TranscriptSegment[] = [];
  for (const s of segments) {
    const last = out[out.length - 1];
    if (last && last.speaker === s.speaker) {
      last.end_ms = s.end_ms;
      last.text = `${last.text} ${s.text}`;
      if (s.confidence !== undefined) {
        last.confidence = Math.min(last.confidence ?? 1, s.confidence);
      }
    } else {
      out.push({ ...s });
    }
  }
  return out;
}

/// `m:ss` from a millisecond offset, for the transcript timestamps.
export function fmtTime(ms: number): string {
  const total = Math.floor(ms / 1000);
  return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, "0")}`;
}

/// A readable speaker name from a diarization label ("speaker_0" -> "Speaker 1").
export function speakerName(label: string | undefined): string {
  if (!label) return "Speaker";
  const m = label.match(/(\d+)$/);
  return m ? `Speaker ${Number(m[1]) + 1}` : label;
}

/// Open the produced note in the text editor (the KG-citizen handoff seam).
export function openInEditor(): void {
  invoke("open_file", { file: "meeting-note.md" }).catch(() => {});
}
