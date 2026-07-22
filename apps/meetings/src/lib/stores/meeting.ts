/// The active meeting: the human's own notes (the Granola anchor - held by this app,
/// what you jotted during capture) plus the produced MeetingNote (the engine output).
/// The verifiable merge shows your notes against the AI summary, both checkable against
/// the embedded transcript.
///
/// Mock-vs-live: fixture-backed. The ASR/diarization capture stream, the `summarize`
/// engine call, the KG-file store, and the text-editor handoff are coder seams; under
/// vite the store serves a fixture so the surface renders.
import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { MeetingNote, Transcript, TranscriptSegment } from "$lib/contract";

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

/// One row in the recent-meetings home. Not in the note contract - a summary the
/// `meetings_list` seam derives from the KG meeting nodes (flagged for the coder).
export interface MeetingSummary {
  id: string;
  title: string;
  date_ms: number;
  participants: string[];
  preview: string;
}

export const meetings = writable<MeetingSummary[]>([]);

/// True while the home list is the FIXTURE, not your real meetings. The rows
/// carry titles, dates and named participants, so unlabelled they read as a
/// history of conversations that never happened.
export const meetingsMocked = writable(false);

/// The app lifecycle: nothing yet, a meeting recording, or the produced note.
export type Phase = "idle" | "capturing" | "note";
export const phase = writable<Phase>("idle");

/// Live capture state (the capturing phase): the transcript as it streams in, the
/// notes you type as the anchor, and the elapsed recording time in ms.
export const liveTranscript = writable<Transcript>({ language: "en", segments: [] });
export const liveNotes = writable("");
export const elapsed = writable(0);

const FIXTURE: Meeting = {
  mocked: true,
  humanNotes:
    "why build our own editor: KG-lens\nmeeting notes must stay on-device (Otter lawsuit)\nTim: capture is its own surface, note goes to the editor",
  note: {
    title: "Editor and meeting-notes direction",
    participants: ["You", "Tim"],
    summary:
      "The KG-lens is the reason to build a first-party editor rather than reuse gedit; a plain editor cannot show provenance and project context. Meeting capture stays fully on-device, which is the edge over cloud transcription bots. Capture lives in its own small surface and the resulting note becomes a knowledge-graph file, opened in the editor for follow-up.",
    summary_claims: [
      { text: "The KG-lens is the reason to build a first-party editor rather than reuse gedit.", source_segment: 0 },
      { text: "Meeting capture stays fully on-device, the edge over cloud transcription bots.", source_segment: 2 },
      { text: "Capture lives in its own small surface and the note becomes a knowledge-graph file.", source_segment: 3 },
      { text: "This keeps the whole workflow sovereign end to end." },
    ],
    action_items: [
      { text: "Split capture into its own Meetings surface", owner: "arlen-ui", source_segment: 3 },
      { text: "File the produced note as a knowledge-graph node", owner: "coder", source_segment: 4 },
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

const MEETINGS_FIXTURE: MeetingSummary[] = [
  {
    id: "m-editor",
    title: "Editor and meeting-notes direction",
    date_ms: 1_751_450_400_000,
    participants: ["You", "Tim"],
    preview: "Why the KG-lens justifies a first-party editor; capture stays on-device.",
  },
  {
    id: "m-standup",
    title: "Weekly standup",
    date_ms: 1_751_277_600_000,
    participants: ["You", "Tim", "Coder"],
    preview: "Titlebar bug cleared, task-manager keyboard drive landed, i18n next.",
  },
  {
    id: "m-sovereignty",
    title: "Sovereignty review",
    date_ms: 1_750_845_600_000,
    participants: ["You", "Tim"],
    preview: "Same-uid ambient authority is the core thesis, not a residual to accept.",
  },
];

/// Load the recent meetings for the home (the `meetings_list` seam over the KG meeting
/// nodes; fixture under vite).
export async function loadMeetings(): Promise<void> {
  try {
    meetings.set(await invoke<MeetingSummary[]>("meetings_list"));
    meetingsMocked.set(false);
  } catch {
    meetings.set(MEETINGS_FIXTURE);
    meetingsMocked.set(true);
  }
}

/// Open a past meeting's note (the `meeting_note {id}` seam; the fixture note under
/// vite), landing on the note phase.
export async function openMeeting(id: string): Promise<void> {
  try {
    const note = await invoke<MeetingNote>("meeting_note", { id });
    meeting.set({ humanNotes: "", note, mocked: false });
  } catch {
    meeting.set({ humanNotes: FIXTURE.humanNotes, note: FIXTURE.note, mocked: true });
  }
  phase.set("note");
}

/// A short, locale-aware meeting date for the list (ties into the i18n locale later).
export function fmtDate(ms: number): string {
  return new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" }).format(new Date(ms));
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

/// The 1-based speaker number from a diarization label ("speaker_0" -> 1), or null
/// when there is none. The display name is formatted in the view via the catalog.
export function speakerNum(label: string | undefined): number | null {
  if (!label) return null;
  const m = label.match(/(\d+)$/);
  return m ? Number(m[1]) + 1 : null;
}

/// Open the produced note in the text editor (the KG-citizen handoff seam).
export function openInEditor(): void {
  invoke("open_file", { file: "meeting-note.md" }).catch(() => {});
}

let ticker: ReturnType<typeof setInterval> | null = null;
let streamer: ReturnType<typeof setInterval> | null = null;

function clearTimers(): void {
  if (ticker) clearInterval(ticker);
  if (streamer) clearInterval(streamer);
  ticker = null;
  streamer = null;
}

/// Begin capturing. Live: the ASR feed fills `liveTranscript`; under vite a dev stream
/// reveals the fixture segments one at a time so the surface shows the streaming
/// experience. The recording is on-device and audited (the sovereign frame).
export function startCapture(): void {
  clearTimers();
  liveTranscript.set({ language: "en", segments: [] });
  liveNotes.set("");
  elapsed.set(0);
  phase.set("capturing");
  invoke("meeting_start_capture").catch(() => {});
  ticker = setInterval(() => elapsed.update((e) => e + 1000), 1000);
  const seg = [...FIXTURE.note.transcript.segments];
  let i = 0;
  streamer = setInterval(() => {
    if (i >= seg.length) {
      if (streamer) clearInterval(streamer);
      streamer = null;
      return;
    }
    const next = seg[i++];
    liveTranscript.update((t) => ({ ...t, segments: [...t.segments, next] }));
  }, 1400);
}

/// Stop capturing and produce the note. Live: the summarize seam turns the transcript +
/// your notes into a MeetingNote; under vite it resolves to the fixture note. The notes
/// you typed are carried into the note view (the app holds them; the note never does).
export async function stopCapture(): Promise<void> {
  clearTimers();
  invoke("meeting_stop_capture").catch(() => {});
  const notes = get(liveNotes);
  try {
    const note = await invoke<MeetingNote>("meeting_summarize", {
      transcript: get(liveTranscript),
      humanNotes: notes,
    });
    meeting.set({ humanNotes: notes, note, mocked: false });
  } catch {
    meeting.set({ humanNotes: notes.trim() || FIXTURE.humanNotes, note: FIXTURE.note, mocked: true });
  }
  phase.set("note");
}
