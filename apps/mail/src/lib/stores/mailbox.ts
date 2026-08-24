/// The mailbox model the client stands on. The INTENDED contract - flagged as a
/// coder seam, not invented quietly: `mail_folders() -> Folder[]`,
/// `mail_list(folderId) -> Envelope[]`, `mail_open(id) -> Message` (the existing
/// `mail_read` DTO shape, one shape on the wire). None of the three exists in
/// Rust yet, so live the store answers "no account connected" and under vite a
/// fixture mailbox stands in - marked as an example on the surface, because a
/// mock that reads as real state is a lie with good typography.
///
/// Local actions (read-mark, archive, delete, drafts) apply to the fixture so
/// the whole client drives; live they will ride `mail_move`/`mail_delete`/
/// `mail_send` when the account backend lands.
import { derived, get, writable } from "svelte/store";
import { tauriAvailable } from "$lib/tauri";
import { invoke } from "@tauri-apps/api/core";

/// The message DTO exactly as `mail_read` serialises it (snake_case, no renames).
export type Message = {
  from: string | null;
  subject: string | null;
  date: string | null;
  text: string | null;
  has_html: boolean;
  only_in_text: string[];
  only_in_html: string[];
  refusal: string | null;
  to: string[];
  cc: string[];
  channels: string[];
  attachments: { name: string | null; media_type: string | null; bytes: number }[];
  invitation: { method: string | null; bytes: number; filename: string | null } | null;
  sealed: string | null;
  path: string;
};

/// The five standard folders. `kind` picks the icon and the catalogue name; a
/// later account backend may add named folders beside them.
export type FolderKind = "inbox" | "sent" | "drafts" | "archive" | "trash";

/// One folder in the rail.
export interface Folder {
  id: string;
  kind: FolderKind;
  unread: number;
}

/// One list row: enough to pick a message, never the message itself.
export interface Envelope {
  id: string;
  folderId: string;
  from: string;
  subject: string;
  snippet: string;
  /// Sent time as epoch milliseconds, for reader-language formatting.
  dateMs: number;
  unread: boolean;
}

// ---------------------------------------------------------------------------
// Fixture: a small, plausible mailbox in English. Deliberately not translated -
// translating fake mail would make it read as a reading of real mail; the
// surface carries an example banner instead. The bodies cover every notice the
// reading surface can show: a divergence, report-back headers, an invitation,
// attachments, a sealed message.
// ---------------------------------------------------------------------------

/// How the sample mailbox writes its own owner. Named rather than repeated, so
/// the draft the compose flow saves borrows the FIXTURE's voice explicitly
/// instead of putting an English word in live code and hoping a reader spots
/// which side of the line it is on.
const FIXTURE_SELF = "You";

const DAY = 86_400_000;
const now = Date.now();

function msg(partial: Partial<Message>): Message {
  return {
    from: null,
    subject: null,
    date: null,
    text: null,
    has_html: false,
    only_in_text: [],
    only_in_html: [],
    refusal: null,
    to: ["you@arlen.local"],
    cc: [],
    channels: [],
    attachments: [],
    invitation: null,
    sealed: null,
    path: "",
    ...partial,
  };
}

const FIXTURE_FOLDERS: Folder[] = [
  { id: "inbox", kind: "inbox", unread: 3 },
  { id: "sent", kind: "sent", unread: 0 },
  { id: "drafts", kind: "drafts", unread: 0 },
  { id: "archive", kind: "archive", unread: 0 },
  { id: "trash", kind: "trash", unread: 0 },
];

const FIXTURE_ENVELOPES: Envelope[] = [
  {
    id: "m1",
    folderId: "inbox",
    from: "Mara Winter",
    subject: "Rehearsal moved to Thursday",
    snippet: "Short version: the hall is free Thursday at seven, so we take it.",
    dateMs: now - 2 * 3600_000,
    unread: true,
  },
  {
    id: "m2",
    folderId: "inbox",
    from: "Vereinsbank Service",
    subject: "Your statement is ready",
    snippet: "The statement for July is attached as a PDF document.",
    dateMs: now - 6 * 3600_000,
    unread: true,
  },
  {
    id: "m3",
    folderId: "inbox",
    from: "Jonas Feld",
    subject: "Invitation: Planning breakfast",
    snippet: "Bringing the roadmap printouts, someone else brings coffee.",
    dateMs: now - DAY,
    unread: true,
  },
  {
    id: "m4",
    folderId: "inbox",
    from: "release-watch",
    subject: "wlroots 0.19 tagged",
    snippet: "The release you subscribed to was tagged an hour ago.",
    dateMs: now - DAY - 5 * 3600_000,
    unread: false,
  },
  {
    id: "m5",
    folderId: "inbox",
    from: "Old Colleague",
    subject: "Long time - quick question",
    snippet: "The text and the formatted part of this one do not say the same thing.",
    dateMs: now - 2 * DAY,
    unread: false,
  },
  {
    id: "m6",
    folderId: "inbox",
    from: "K. Adler",
    subject: "Sealed note",
    snippet: "This message is encrypted with PGP.",
    dateMs: now - 3 * DAY,
    unread: false,
  },
  {
    id: "m7",
    folderId: "sent",
    from: "You",
    subject: "Re: Rehearsal moved to Thursday",
    snippet: "Thursday works. I will bring the recordings from last time.",
    dateMs: now - 90 * 60_000,
    unread: false,
  },
  {
    id: "m8",
    folderId: "sent",
    from: "You",
    subject: "Minutes from Monday",
    snippet: "As promised, the notes - corrections welcome until Friday.",
    dateMs: now - 4 * DAY,
    unread: false,
  },
  {
    id: "m9",
    folderId: "drafts",
    from: "You",
    subject: "Draft: birthday plan",
    snippet: "Do not send this before the cake is confirmed.",
    dateMs: now - 5 * DAY,
    unread: false,
  },
  {
    id: "m10",
    folderId: "archive",
    from: "Housing Board",
    subject: "Meter reading confirmed",
    snippet: "Your reading was recorded. No further action is needed.",
    dateMs: now - 20 * DAY,
    unread: false,
  },
  {
    id: "m11",
    folderId: "archive",
    from: "Mara Winter",
    subject: "Photos from the concert",
    snippet: "Three good ones attached, the rest were too dark.",
    dateMs: now - 34 * DAY,
    unread: false,
  },
  {
    id: "m12",
    folderId: "trash",
    from: "newsletter",
    subject: "You may have already won",
    snippet: "This one asks to report back when you read it.",
    dateMs: now - 8 * DAY,
    unread: false,
  },
];

const FIXTURE_MESSAGES: Record<string, Message> = {
  m1: msg({
    from: "Mara Winter <mara@example.org>",
    subject: "Rehearsal moved to Thursday",
    date: new Date(now - 2 * 3600_000).toISOString(),
    text: "Short version: the hall is free Thursday at seven, so we take it.\n\nSame plan as last week - run the second half first while everyone is fresh, then the opening. If Thursday does not work for you, say so today and we fall back to Friday.\n\nMara",
  }),
  m2: msg({
    from: "Vereinsbank Service <service@vereinsbank.example>",
    subject: "Your statement is ready",
    date: new Date(now - 6 * 3600_000).toISOString(),
    text: "The statement for July is attached as a PDF document.\n\nThis mailbox is not read. For questions, use the contact form.",
    has_html: true,
    attachments: [{ name: "statement-july.pdf", media_type: "application/pdf", bytes: 182_444 }],
  }),
  m3: msg({
    from: "Jonas Feld <jonas@example.org>",
    subject: "Invitation: Planning breakfast",
    date: new Date(now - DAY).toISOString(),
    text: "Bringing the roadmap printouts, someone else brings coffee.\n\nSaturday, nine, the usual place. Calendar part attached for whoever keeps one.",
    invitation: { method: "request", bytes: 1_204, filename: "breakfast.ics" },
    attachments: [{ name: "breakfast.ics", media_type: "text/calendar", bytes: 1_204 }],
  }),
  m4: msg({
    from: "release-watch <noreply@releases.example>",
    subject: "wlroots 0.19 tagged",
    date: new Date(now - DAY - 5 * 3600_000).toISOString(),
    text: "The release you subscribed to was tagged an hour ago.\n\nTag: 0.19.0\nCompare: 0.18.2...0.19.0",
  }),
  m5: msg({
    from: "Old Colleague <colleague@example.net>",
    subject: "Long time - quick question",
    date: new Date(now - 2 * DAY).toISOString(),
    text: "Hey! Quick one: do you still have the deploy notes from the old project? No rush.",
    has_html: true,
    only_in_text: ["No rush."],
    only_in_html: ["today", "urgent"],
  }),
  m6: msg({
    from: "K. Adler <k.adler@example.org>",
    subject: "Sealed note",
    date: new Date(now - 3 * DAY).toISOString(),
    sealed: "pgp",
  }),
  m7: msg({
    from: "You",
    subject: "Re: Rehearsal moved to Thursday",
    date: new Date(now - 90 * 60_000).toISOString(),
    to: ["mara@example.org"],
    text: "Thursday works. I will bring the recordings from last time.",
  }),
  m8: msg({
    from: "You",
    subject: "Minutes from Monday",
    date: new Date(now - 4 * DAY).toISOString(),
    to: ["team@example.org"],
    text: "As promised, the notes - corrections welcome until Friday.",
    attachments: [{ name: "minutes-monday.md", media_type: "text/markdown", bytes: 9_812 }],
  }),
  m9: msg({
    from: "You",
    subject: "Draft: birthday plan",
    date: new Date(now - 5 * DAY).toISOString(),
    to: [],
    text: "Do not send this before the cake is confirmed.",
  }),
  m10: msg({
    from: "Housing Board <board@example.org>",
    subject: "Meter reading confirmed",
    date: new Date(now - 20 * DAY).toISOString(),
    text: "Your reading was recorded. No further action is needed.",
  }),
  m11: msg({
    from: "Mara Winter <mara@example.org>",
    subject: "Photos from the concert",
    date: new Date(now - 34 * DAY).toISOString(),
    text: "Three good ones attached, the rest were too dark.",
    attachments: [
      { name: "concert-01.jpg", media_type: "image/jpeg", bytes: 2_411_000 },
      { name: "concert-04.jpg", media_type: "image/jpeg", bytes: 1_988_000 },
      { name: null, media_type: "image/jpeg", bytes: 2_140_000 },
    ],
  }),
  m12: msg({
    from: "newsletter <win@prizes.example>",
    subject: "You may have already won",
    date: new Date(now - 8 * DAY).toISOString(),
    text: "Click now to claim your prize before it expires!",
    has_html: true,
    channels: ["disposition-notification-to", "x-image-url"],
  }),
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The folders in the rail; empty means no account is connected.
export const folders = writable<Folder[]>([]);
/// Every envelope the store knows, across folders; the page slices per folder.
export const envelopes = writable<Envelope[]>([]);
/// True while the mailbox is the FIXTURE - the surface says so.
export const mailboxMocked = writable(false);
/// The inbox unread count, derived so the rail never counts by hand.
export const unreadCount = derived(envelopes, ($e) => $e.filter((x) => x.unread && x.folderId === "inbox").length);

/// Load folders and envelopes. Live: the intended `mail_folders`/`mail_list`
/// commands; until they exist the catch answers with the fixture under vite
/// and with an empty, honestly-unconnected mailbox on a real host.
export async function loadMailbox(): Promise<void> {
  try {
    const f = await invoke<Folder[]>("mail_folders");
    const lists = await Promise.all(f.map((x) => invoke<Envelope[]>("mail_list", { folderId: x.id })));
    folders.set(f);
    envelopes.set(lists.flat());
    mailboxMocked.set(false);
  } catch {
    if (tauriAvailable) {
      // A real host with no mailbox backend: an unconnected mailbox, not a
      // pretend one. The launch path (`mail_read`) still works beside this.
      folders.set([]);
      envelopes.set([]);
      mailboxMocked.set(false);
    } else {
      folders.set(structuredClone(FIXTURE_FOLDERS));
      envelopes.set(structuredClone(FIXTURE_ENVELOPES));
      mailboxMocked.set(true);
    }
  }
}

/// Open one message. Live: the intended `mail_open`; fixture: the local map.
///
/// THE SAME SPLIT AS `loadMailbox`, which this was missing. A real session whose
/// open failed got a fixture message back - somebody else's words, rendered as
/// the message you clicked. Under a host the answer is now nothing, and the
/// surface says the message could not be opened; the sample only stands in where
/// there is no host to have asked.
export async function openMessage(id: string): Promise<Message | null> {
  try {
    return await invoke<Message>("mail_open", { id });
  } catch {
    return tauriAvailable ? null : (FIXTURE_MESSAGES[id] ?? null);
  }
}

/// Selecting a message marks it read; local until the backend owns flags.
export function markRead(id: string): void {
  envelopes.update((all) => all.map((e) => (e.id === id ? { ...e, unread: false } : e)));
}

/// Move a message to another folder (archive, trash). Local on the fixture;
/// rides `mail_move` when it exists.
export function moveMessage(id: string, folderId: string): void {
  envelopes.update((all) => all.map((e) => (e.id === id ? { ...e, folderId, unread: false } : e)));
}

/// Delete from the trash: the row is gone for good. Everywhere else deleting
/// MOVES to trash (undo by moving back), which is the reversible default.
export function deleteForever(id: string): void {
  envelopes.update((all) => all.filter((e) => e.id !== id));
}

/// A draft saved from compose: prepended to Drafts so the flow completes.
export function saveDraft(to: string, subject: string, body: string): string {
  const id = `draft-${Date.now()}`;
  const env: Envelope = {
    id,
    folderId: "drafts",
    from: FIXTURE_SELF,
    // EMPTY, not a word. A subject the reader sees is the reader's language, and
    // this store cannot know it; the list writes "(kein Betreff)" itself when
    // there is nothing to show.
    subject,
    snippet: body.split("\n")[0] ?? "",
    dateMs: Date.now(),
    unread: false,
  };
  FIXTURE_MESSAGES[id] = msg({
    from: "You",
    subject: env.subject,
    date: new Date(env.dateMs).toISOString(),
    to: to ? to.split(",").map((s) => s.trim()) : [],
    text: body,
  });
  envelopes.update((all) => [env, ...all]);
  return id;
}

/// Whether the sender is a person this machine already knows. The intended
/// command is `mail_sender_person(address)` reading `shared.Person` from the
/// Knowledge Graph (contacts-decision.md: mail READS people, it never owns
/// them). Fixture: one known sender, so the affordance is visible.
///
/// UNDER A HOST, NOBODY. The fixture used to answer here whatever went wrong, so
/// a real machine reading a message from that one address showed a resolved
/// person the graph had never been asked about - a claim that this desktop knows
/// who wrote to you. It knows nothing until contacts exist; saying so is the
/// whole affordance.
export async function senderPerson(address: string): Promise<{ name: string } | null> {
  try {
    return await invoke<{ name: string } | null>("mail_sender_person", { address });
  } catch {
    if (tauriAvailable) return null;
    return address === "mara@example.org" ? { name: "Mara Winter" } : null;
  }
}

/// The message a launch handed over (`launch_file` + `mail_read`) - kept beside
/// the mailbox because it belongs to no folder.
export const openedFile = writable<Message | null>(null);
