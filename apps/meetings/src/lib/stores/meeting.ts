/// The active meeting: the human's own notes (the Granola anchor - what you jotted
/// during capture) plus the produced MeetingNote (the engine output). The note view
/// merges the two INLINE - your lines full-strength, the AI's enhancements in the AI
/// tint under the line they anchor to - both checkable against the embedded transcript.
///
/// Mock-vs-live: `meetings_list`, `meeting_note {id}` and `meeting_summarize` are
/// live; the ASR capture stream, notes persistence (`meeting_save_notes`), speaker
/// relabel (`meeting_relabel_speaker`) and item updates (`meeting_update_item`) are
/// coder seams - under vite the fixture stands in and edits apply locally.
import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { locale } from "$lib/i18n/messages";
import type { MeetingNote, Transcript, TranscriptSegment } from "$lib/contract";

/// A captured meeting: your notes (the anchor) + the produced note.
export interface Meeting {
  /// What you typed during the meeting - the anchor that steers salience. Held
  /// by the app, never sent back inside the MeetingNote.
  humanNotes: string;
  /// The engine's output (summary + action items + transcript).
  note: MeetingNote;
  /// True while showing the fixture (no engine under vite).
  mocked: boolean;
}

export const meeting = writable<Meeting | null>(null);
/// The id of the open meeting ("live" right after a capture, before the KG files it).
export const currentId = writable<string | null>(null);

/// User corrections of diarization labels ("speaker_0" -> "Anna"). A draft the
/// user confirms - diarization is ~20% wrong on real meetings, so relabelling is
/// first-class, never buried.
export const speakerNames = writable<Record<string, string>>({});

/// One row in the recent-meetings home. Not in the note contract - a summary the
/// `meetings_list` seam derives from the KG meeting nodes.
export interface MeetingSummary {
  id: string;
  title: string;
  date_ms: number;
  participants: string[];
  preview: string;
}

export const meetings = writable<MeetingSummary[]>([]);

/// True while the home list is the FIXTURE, not your real meetings.
export const meetingsMocked = writable(false);

/// True when a real session could not read the meetings at all.
///
/// Separate from `meetingsMocked`, and the separation is the whole point: one
/// says "these are examples", the other says "there is nothing here and that is
/// not a statement about your meetings". Until 9 August a real session with no
/// engine got the fixture and the label, which reads as the first when it is the
/// second - and the rows carry actions. A label does not rescue a fixture you can
/// click: the click either does nothing, which is a lie about the control, or it
/// does something to invented data, which is worse. Printers, clock and files all
/// answered this the same way and for the same reason.
export const meetingsUnavailable = writable(false);

/// True when a real session could not read the open meeting's note.
export const noteUnavailable = writable(false);

/// Live capture state: the transcript as it streams in, the notes you type as the
/// anchor, whether transcription is on (a separate, opt-outable step - recording
/// and transcribing are different consents), and the elapsed time in ms.
export const liveTranscript = writable<Transcript>({ language: "en", segments: [] });
export const liveNotes = writable("");
export const transcribe = writable(true);
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
      { text: "A plain editor cannot surface provenance or project context - that is what the lens adds.", source_segment: 1, anchor_line: 0 },
      { text: "On-device capture is the edge over cloud transcription bots - the Otter class action is the trap avoided.", source_segment: 2, anchor_line: 1 },
      { text: "The produced note becomes a knowledge-graph file, opened in the editor for follow-up.", source_segment: 3, anchor_line: 2 },
      { text: "This keeps the whole workflow sovereign end to end." },
    ],
    action_items: [
      { text: "Split capture into its own Meetings surface", owner: "arlen-ui", source_segment: 3 },
      { text: "File the produced note as a knowledge-graph node", source_segment: 4 },
    ],
    transcript: {
      language: "en",
      segments: [
        { start_ms: 4200, end_ms: 9800, speaker: "speaker_0", confidence: 0.95, text: "So the whole reason to build our own editor is the knowledge-graph lens." },
        { start_ms: 9800, end_ms: 15200, speaker: "speaker_0", confidence: 0.93, text: "A plain editor like gedit just cannot surface provenance or which project a file belongs to." },
        { start_ms: 15200, end_ms: 21000, speaker: "speaker_1", confidence: 0.72, text: "Right, and the meeting notes have to stay on this device. The Otter lawsuit is exactly the trap we avoid." },
        { start_ms: 21000, end_ms: 27400, speaker: "speaker_1", confidence: 0.92, text: "Let's make the capture its own small surface, and the note it produces becomes a graph file you open in the editor." },
        { start_ms: 27400, end_ms: 31900, speaker: "speaker_0", confidence: 0.94, text: "Agreed. Capture is one lifecycle, the note is a citizen of the graph after." },
      ],
    },
  },
};

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

/// Load the recent meetings for the home (live: KG meeting nodes; fixture under vite).
export async function loadMeetings(): Promise<void> {
  try {
    meetings.set(await invoke<MeetingSummary[]>("meetings_list"));
    meetingsMocked.set(false);
    meetingsUnavailable.set(false);
  } catch {
    if (import.meta.env.DEV) {
      meetings.set(MEETINGS_FIXTURE);
      meetingsMocked.set(true);
      meetingsUnavailable.set(false);
      return;
    }
    meetings.set([]);
    meetingsMocked.set(false);
    meetingsUnavailable.set(true);
  }
}

/// Open a past meeting's note by id (live: `meeting_note {id}`; the fixture under
/// vite). The route mounts call this; navigation is the router's job.
export async function openMeeting(id: string): Promise<void> {
  currentId.set(id);
  speakerNames.set({});
  try {
    const note = await invoke<MeetingNote>("meeting_note", { id });
    // Without the user's own lines the two-voice merge silently degrades to a
    // flat AI summary, so load them alongside the note; an app that never
    // stored any gets an empty string either way.
    let humanNotes = "";
    try {
      humanNotes = await invoke<string>("meeting_human_notes", { id });
    } catch {
      // The note still renders; every claim falls into the unanchored bucket.
    }
    meeting.set({ humanNotes, note, mocked: false });
    noteUnavailable.set(false);
  } catch {
    if (import.meta.env.DEV) {
      meeting.set({ humanNotes: FIXTURE.humanNotes, note: FIXTURE.note, mocked: true });
      noteUnavailable.set(false);
      return;
    }
    // The note page is participants, claims and a transcript. Serving the fixture
    // one here put invented quotes under a real meeting's id, and the page's own
    // edit controls then wrote against that id.
    meeting.set(null);
    noteUnavailable.set(true);
  }
}

/// True when the last edit did not reach the host. Everything else on this
/// surface is a labelled sample, but the notes, the speaker names and the action
/// items are the user's OWN words - losing them silently is the one failure this
/// app cannot absorb behind a caveat.
export const editFailed = writable(false);

/// Save the user's edited notes.
export async function saveNotes(text: string): Promise<void> {
  const before = get(meeting)?.humanNotes;
  meeting.update((m) => (m ? { ...m, humanNotes: text } : m));
  editFailed.set(false);
  try {
    await invoke("meeting_save_notes", { id: get(currentId), text });
  } catch {
    if (import.meta.env.DEV) return; // no host under vite
    // Showing the text as saved is how somebody closes the window and loses it.
    meeting.update((m) => (m ? { ...m, humanNotes: before ?? "" } : m));
    editFailed.set(true);
  }
}

/// Relabel a diarization speaker everywhere ("speaker_1" -> "Ben"). Confirming
/// attribution is the user's job by design; the seam persists it with the note.
export async function relabelSpeaker(label: string, name: string): Promise<void> {
  const beforeNames = get(speakerNames);
  editFailed.set(false);
  speakerNames.update((s) => {
    const next = { ...s };
    if (name.trim()) next[label] = name.trim();
    else delete next[label];
    return next;
  });
  try {
    await invoke("meeting_relabel_speaker", { id: get(currentId), label, name: name.trim() });
  } catch {
    if (import.meta.env.DEV) return;
    speakerNames.set(beforeNames);
    editFailed.set(true);
  }
}

/// Update one action item (owner confirm/edit, done tick). Persisted via the seam.
export async function updateItem(index: number, patch: { owner?: string; done?: boolean }): Promise<void> {
  const beforeMeeting = get(meeting);
  editFailed.set(false);
  meeting.update((m) => {
    if (!m) return m;
    const items = m.note.action_items.map((it, i) => (i === index ? { ...it, ...patch } : it));
    return { ...m, note: { ...m.note, action_items: items } };
  });
  try {
    await invoke("meeting_update_item", { id: get(currentId), index, ...patch });
  } catch {
    if (import.meta.env.DEV) return;
    // A tick that did not persist means the item comes back undone next time,
    // and the owner shown against it is not the one recorded.
    meeting.set(beforeMeeting);
    editFailed.set(true);
  }
}

/// A short meeting date for the list.
export function fmtDate(ms: number, loc = get(locale)): string {
  // The chosen language, not `undefined` - which means the system locale, and
  // that is a different setting the user did not touch here.
  //
  // The locale is a parameter, not just a read: a template calling `fmtDate(x)`
  // has no reactive dependency on the store, so it renders once with whatever
  // was current and keeps it. Passing `$locale` at the call site is what makes
  // the list re-render when the language changes - and what stops it showing
  // "Jul 2" under a German heading because the read beat the startup fetch.
  return new Intl.DateTimeFormat(loc, { day: "numeric", month: "short" }).format(new Date(ms));
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

/// Open the produced note in the text editor (the KG-citizen handoff seam; the
/// real note path is the coder's to supply).
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

/// Begin capturing. Live: the ASR feed fills `liveTranscript` (when transcription
/// is on); under vite a dev stream reveals the fixture segments so the streaming
/// experience shows. On-device, nothing joins the call.
export function startCapture(): void {
  clearTimers();
  liveTranscript.set({ language: "en", segments: [] });
  liveNotes.set("");
  transcribe.set(true);
  elapsed.set(0);
  invoke("meeting_start_capture").catch(() => {});
  ticker = setInterval(() => elapsed.update((e) => e + 1000), 1000);
  const seg = [...FIXTURE.note.transcript.segments];
  let i = 0;
  streamer = setInterval(() => {
    if (!get(transcribe)) return;
    if (i >= seg.length) {
      if (streamer) clearInterval(streamer);
      streamer = null;
      return;
    }
    const next = seg[i++];
    liveTranscript.update((t) => ({ ...t, segments: [...t.segments, next] }));
  }, 1400);
}

/// Stop capturing and produce the note. Live: the summarize seam turns the
/// transcript + your notes into a MeetingNote; under vite it resolves to the
/// fixture. The caller navigates to the note route after.
export async function stopCapture(): Promise<boolean> {
  clearTimers();
  invoke("meeting_stop_capture").catch(() => {});
  const notes = get(liveNotes);
  currentId.set("live");
  speakerNames.set({});
  try {
    const note = await invoke<MeetingNote>("meeting_summarize", {
      transcript: get(liveTranscript),
      humanNotes: notes,
    });
    meeting.set({ humanNotes: notes, note, mocked: false });
    noteUnavailable.set(false);
    return true;
  } catch {
    if (import.meta.env.DEV) {
      meeting.set({ humanNotes: notes.trim() || FIXTURE.humanNotes, note: FIXTURE.note, mocked: true });
      noteUnavailable.set(false);
      return true;
    }
    // The sharpest of the three fixture paths, because of when it fires: the
    // user has just recorded a meeting and typed their own notes, and a failed
    // summarise handed back invented participants and invented quotes with those
    // real notes attached. `liveNotes` is deliberately not cleared - whatever
    // they typed is theirs and still on the capture surface - and the false
    // return tells the caller not to navigate to a note that does not exist.
    meeting.set(null);
    noteUnavailable.set(true);
    return false;
  }
}
