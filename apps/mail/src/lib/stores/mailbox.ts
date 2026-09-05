/// The mailbox model the client stands on. Reading is live since 4 September:
/// `mail_folders() -> Folder[]`, `mail_list(folderId) -> Envelope[]` and
/// `mail_open(id) -> Message` (the `mail_read` DTO shape, one shape on the wire)
/// read this machine's maildir. Under vite a fixture mailbox stands in, marked as
/// an example on the surface, because a mock that reads as real state is a lie
/// with good typography.
///
/// WRITING IS NOT LIVE. Archive, delete, drafts and the read mark are local
/// mutations that the fixture keeps and a maildir does not - a file that was
/// "archived" is back in the inbox at the next start. So the surface offers the
/// writes only while the mailbox is the sample (`mailboxWritable`), and live it
/// is a reader until `mail_move`/`mail_delete`/`mail_draft_save`/`mail_mark_seen`
/// exist. The read mark is the one exception: the dot clears on reading, as in
/// every client, and the flag write is the seam that makes it stick.
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
  /// What the surface calls this message, and it does not change while the
  /// window is open. THIS IS NOT THE FILE. A maildir keeps a message's flags in
  /// its filename, so reading one renames it - and an id that changes under the
  /// selection holding it is not an id: the first cut updated the row in place
  /// and the toolbar vanished the moment somebody clicked a message, because the
  /// selected set still held the name the file had a second ago.
  id: string;
  /// Where the message is in the mailbox right now, when that has come apart
  /// from what it is called. Absent until the first write moves it, so a row
  /// that has not been touched carries one name and not two.
  place?: string;
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

/// The folders in the rail; empty means there is no maildir to read.
export const folders = writable<Folder[]>([]);
/// Every envelope the store knows, across folders; the page slices per folder.
export const envelopes = writable<Envelope[]>([]);

/// Where the mailbox stands: still being read, read from this machine, absent,
/// unreadable, or the sample under vite. Distinct from the folders being empty,
/// because "nothing answered yet" and "there is nothing to read" are different
/// sentences and only one of them is true while a big maildir loads.
///
/// `absent` and `unreadable` were one state until 5 September, and it said "no
/// account is connected" for both. Nothing in this system has accounts - the
/// host reads a maildir at a path - so the sentence sent a reader looking for a
/// setting that does not exist. They are two facts with two different things to
/// do about them: put a maildir where the host looks, or find out why the host
/// could not answer.
export type MailboxState = "loading" | "live" | "absent" | "unreadable" | "sample";
export const mailboxState = writable<MailboxState>("loading");
/// Where this machine keeps mail, as `mail_store` writes it (`~/Maildir`). Null
/// until the host says, and on a host that has no home to look in.
export const mailboxRoot = writable<string | null>(null);
/// True while the mailbox is the FIXTURE - the surface says so.
export const mailboxMocked = derived(mailboxState, ($s) => $s === "sample");
/// Whether the mailbox can KEEP a write - archive, delete, the read mark, a
/// draft. The sample keeps them in memory; a maildir keeps them on disk, since
/// 5 September, because each of those four is a rename or a file
/// (`mail_mark_seen`, `mail_move`, `mail_delete`, `mail_draft_save`). It was
/// sample-only while they were pretences: a message "archived" into a store and
/// back in the inbox at the next start is worse than a control that is not there.
export const mailboxWritable = derived(mailboxState, ($s) => $s === "sample" || $s === "live");
/// Whether STARTING a message is on offer, which is a different question and has
/// a different answer. Writing a draft works on a maildir; sending needs an
/// account and Arlen has no account surface anywhere, so `mail-app.md` rules
/// Compose stays absent live and no line explains the absence. Reply and Forward
/// are not this: they answer a message that is in front of you, and they land in
/// the same drafts folder the ruling names.
export const mailboxComposes = derived(mailboxState, ($s) => $s === "sample");
/// The inbox unread count, derived so the rail never counts by hand.
export const unreadCount = derived(envelopes, ($e) => $e.filter((x) => x.unread && x.folderId === "inbox").length);

/// Load folders and envelopes. Live: `mail_folders`, then one `mail_list` per
/// folder. The rail stands as soon as the folders are known and the state stays
/// `loading` until the rows are in, so a large maildir shows "reading" rather
/// than an empty mailbox for the seconds it takes. The catch answers with the
/// fixture under vite and an unreadable mailbox on a host.
export async function loadMailbox(): Promise<void> {
  mailboxState.set("loading");
  let f: Folder[];
  try {
    f = await invoke<Folder[]>("mail_folders");
  } catch {
    if (tauriAvailable) {
      // The host is there and did not answer: a fault to report, not an empty
      // mailbox. The launch path (`mail_read`) still works beside this.
      folders.set([]);
      envelopes.set([]);
      mailboxState.set("unreadable");
    } else {
      folders.set(structuredClone(FIXTURE_FOLDERS));
      envelopes.set(structuredClone(FIXTURE_ENVELOPES));
      mailboxState.set("sample");
    }
    return;
  }
  if (f.length === 0) {
    // The host answered and there is no maildir. `folders` yields the inbox for
    // any real one, so an empty list means exactly that and nothing else. Ask
    // where it looked, so the window can name the place instead of the reader
    // guessing at it.
    folders.set([]);
    envelopes.set([]);
    try {
      mailboxRoot.set(await invoke<string | null>("mail_store"));
    } catch {
      mailboxRoot.set(null);
    }
    mailboxState.set("absent");
    return;
  }
  folders.set(f);
  const lists = await Promise.all(
    f.map(async (x) => {
      try {
        // The id a row arrives with IS its maildir path; `place` only appears
        // once a write moves the file out from under it.
        return await invoke<Envelope[]>("mail_list", { folderId: x.id });
      } catch {
        // One folder that would not list is an empty folder, not a dead mailbox.
        return [];
      }
    }),
  );
  envelopes.set(lists.flat());
  mailboxState.set("live");
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
    return await invoke<Message>("mail_open", { id: placeOf(id) ?? id });
  } catch {
    return tauriAvailable ? null : (FIXTURE_MESSAGES[id] ?? null);
  }
}

/// The folder a caller named, by its id or by its kind.
///
/// The two coincide on the fixture and never live: the sample's ids ARE the
/// kinds (`archive`, `trash`), a maildir's are its directory names (`.Archive`).
/// A surface that says "archive this" means the rail, not a path, so the store
/// resolves it and the page keeps saying what it means.
/// Where the message a surface calls `id` currently is.
function placeOf(id: string): string | undefined {
  const row = get(envelopes).find((e) => e.id === id);
  return row && (row.place ?? row.id);
}

function folderNamed(target: string): Folder | undefined {
  const all = get(folders);
  return all.find((f) => f.id === target) ?? all.find((f) => f.kind === target);
}

/// Mark a message read.
///
/// THE ID CHANGES LIVE, because a maildir message's flags are in its filename.
/// The row carries the new one, or the write did not happen and the row is left
/// exactly as it was - a dot cleared over a failed rename is the optimistic
/// write this tree has a gate against, and here it would mean a message the
/// person believes they have read coming back unread at the next start.
export async function markRead(id: string): Promise<void> {
  if (get(mailboxState) === "live") {
    const at = placeOf(id);
    if (!at) return;
    let next: string;
    try {
      next = await invoke<string>("mail_mark_seen", { id: at });
    } catch {
      return;
    }
    envelopes.update((all) =>
      all.map((e) => (e.id === id ? { ...e, place: next, unread: false } : e)),
    );
    return;
  }
  envelopes.update((all) => all.map((e) => (e.id === id ? { ...e, unread: false } : e)));
}

/// Move a message to another folder, named by rail or by id.
///
/// Nothing moves on the surface until the mailbox says it moved.
export async function moveMessage(id: string, target: string): Promise<void> {
  if (get(mailboxState) === "live") {
    const dest = folderNamed(target);
    const at = placeOf(id);
    if (!dest || !at) return;
    let next: string;
    try {
      next = await invoke<string>("mail_move", { id: at, folderId: dest.id });
    } catch {
      return;
    }
    envelopes.update((all) =>
      all.map((e) => (e.id === id ? { ...e, place: next, folderId: dest.id } : e)),
    );
    return;
  }
  envelopes.update((all) => all.map((e) => (e.id === id ? { ...e, folderId: target, unread: false } : e)));
}

/// Delete a message, by the mailbox's own rule: to the trash if there is one and
/// it is not already there, off the disk if not.
///
/// THE POLICY IS HERE RATHER THAN ON THE SURFACE, which used to decide it by
/// comparing the open folder against the string "trash" - true of the sample,
/// never of a maildir, whose trash is called `.Trash`. Live that comparison
/// failed every time and a delete from the trash quietly moved the message to
/// where it already was.
export async function deleteMessage(id: string): Promise<void> {
  if (get(mailboxState) === "live") {
    const at = placeOf(id);
    if (!at) return;
    let landed: string | null;
    try {
      landed = await invoke<string | null>("mail_delete", { id: at });
    } catch {
      return;
    }
    if (landed === null) {
      envelopes.update((all) => all.filter((e) => e.id !== id));
      return;
    }
    const trash = get(folders).find((f) => f.kind === "trash");
    const next = landed;
    envelopes.update((all) =>
      all.map((e) => (e.id === id ? { ...e, place: next, folderId: trash?.id ?? e.folderId } : e)),
    );
    return;
  }
  const here = get(envelopes).find((e) => e.id === id);
  if (here && folderNamed(here.folderId)?.kind === "trash") {
    envelopes.update((all) => all.filter((e) => e.id !== id));
    return;
  }
  await moveMessage(id, "trash");
}

/// A draft saved from compose: prepended to Drafts so the flow completes.
export function saveDraftLocal(to: string, subject: string, body: string): string {
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

/// Save a draft, and answer with where it went.
///
/// Live it is a file in the mailbox's drafts folder, written by `mail_draft_save`
/// - which CREATES that folder if the maildir had none, the one place these
/// writes make a directory, because a draft has nowhere else to be. The mailbox
/// is re-read afterwards rather than a row invented here: the folder may not have
/// been in the rail a moment ago, and the id is the file's name.
///
/// A FAILED SAVE ANSWERS NULL and adds nothing, so the composer can say the
/// draft is not kept rather than close over a message that went nowhere.
export async function saveDraft(to: string, subject: string, body: string): Promise<string | null> {
  if (get(mailboxState) === "live") {
    let id: string;
    try {
      id = await invoke<string>("mail_draft_save", { to, subject, body });
    } catch {
      return null;
    }
    await loadMailbox();
    return id;
  }
  return saveDraftLocal(to, subject, body);
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
