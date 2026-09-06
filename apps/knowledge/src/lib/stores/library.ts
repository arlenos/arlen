/// The Library model (knowledge-app.md §3b, decision 7): the bridged knowledge
/// content, browsed BY SOURCE, each section carrying its origin tag - the same
/// origin a per-source revoke severs.
///
/// **Nothing here decides what a section is called or how it is laid out.** The
/// type declares that in its own schema and the daemon resolves it
/// (bridge-architecture.md, 9 August): a display name, a class from a closed set,
/// and which field is the title. This store had four hardcoded section keys -
/// papers, books, notes, mail - which is the frontend lookup table that decision
/// ruled out: it means a new connector needs a change to the desktop, and a
/// third-party bridge would ship a type this page could not name.
import { writable } from "svelte/store";
import { tauriAvailable } from "$lib/tauri";
import { isServiceAbsent } from "$lib/service";
import { invoke } from "@tauri-apps/api/core";

/// One bridged item, display-shaped.
export interface LibraryEntry {
  id: string;
  /// Already resolved by the daemon: the declared title field, or the entry's
  /// own key when the type declared none. Never a field nobody named.
  title: string;
  /// The declared second line. Null when the type declared none, so the row
  /// leaves the column empty rather than rendering a gap under a blank string.
  sub: string | null;
  /// Seconds since the epoch: **when this machine last wrote the row.** Not when
  /// the item was published, received or edited - no bridge declares that today -
  /// so this reads as "added" and the page must not say otherwise.
  added: number | null;
}

/// How the interface lays a section out. The closed set, mirroring the daemon's.
export type DisplayClass = "notes" | "documents" | "messages" | "media" | "other";

/// One section: one declared type, from one source.
export interface LibrarySection {
  /// The namespace, which is the origin tag shown quietly in the head.
  source: string;
  /// The fully qualified type.
  type: string;
  /// What to call it: the declared display name, or the qualified type when the
  /// type declared none. Not translated here - the name belongs to whoever
  /// defined the type, which for a third-party bridge is not this repository.
  label: string;
  class: DisplayClass;
  entries: LibraryEntry[];
}

/// The order sections appear in. A stable order the interface owns, so adding a
/// bridge never reorders the page and a class this build has never seen still has
/// a place - which is the whole reason the set is closed.
const CLASS_ORDER: DisplayClass[] = ["notes", "documents", "messages", "media", "other"];

/// Sections by class, then by source, so the order is the same on every read.
export function sortSections(sections: LibrarySection[]): LibrarySection[] {
  const rank = (c: DisplayClass): number => {
    const i = CLASS_ORDER.indexOf(c);
    return i === -1 ? CLASS_ORDER.length : i;
  };
  return [...sections].sort(
    (a, b) => rank(a.class) - rank(b.class) || a.source.localeCompare(b.source),
  );
}

/// True while the sections are the FIXTURE, not the graph.
export const libraryMocked = writable(false);

/// True when a real session could not read the library at all.
export const libraryUnavailable = writable(false);

/// True when the knowledge service is not running, which is not a failed read.
/// A fresh machine that has not started the daemon is the ordinary case, and
/// "cannot read your library right now" promises a retry that will not help.
export const libraryNoService = writable(false);

/// The loaded sections, or null before the read settles.
export const sources = writable<LibrarySection[] | null>(null);

const now = Math.floor(Date.now() / 1000);
const daysAgo = (d: number, h = 12): number => {
  const dd = new Date(now * 1000);
  dd.setHours(h, 0, 0, 0);
  return Math.floor(dd.getTime() / 1000) - d * 86400;
};

// i18n-foreign: the user's own documents - note names, paper titles, mail
// subjects. Content, not interface. Translating it would be a mistranslation.
//
// The last section is deliberately a type that declared no display name and no
// title field, so the fixture shows what the fallbacks look like rather than only
// the happy path: the head reads as the qualified type and the rows read as their
// own keys.
const FIXTURE: LibrarySection[] = [
  {
    source: "md.obsidian",
    type: "md.obsidian.Note",
    label: "Notes",
    class: "notes",
    entries: [
      { id: "n-1", title: "Deep work", sub: "Reading notes", added: daysAgo(2, 9) },
      { id: "n-2", title: "Thesis outline", sub: "Thesis", added: daysAgo(5, 16) },
    ],
  },
  {
    source: "org.zotero",
    type: "org.zotero.Paper",
    label: "Papers",
    class: "documents",
    entries: [
      { id: "p-1", title: "Attention Is All You Need", sub: "Vaswani et al, 2017", added: daysAgo(1, 13) },
      { id: "p-2", title: "Scaling Laws for Neural Language Models", sub: "Kaplan et al, 2020", added: daysAgo(6, 9) },
    ],
  },
  {
    source: "org.mozilla.thunderbird",
    type: "org.mozilla.thunderbird.Message",
    label: "Mail",
    class: "messages",
    entries: [
      { id: "m-1", title: "Re: review notes", sub: "alex@example.com", added: daysAgo(3, 8) },
    ],
  },
  {
    source: "com.example.scanner",
    type: "com.example.scanner.Scan",
    label: "com.example.scanner.Scan",
    class: "other",
    entries: [{ id: "s-1", title: "scans/2026-08-14-001.tiff", sub: null, added: daysAgo(9, 11) }],
  },
];

/// Load the sections. Live: `knowledge_library`; fixture under vite.
export async function loadLibrary(): Promise<void> {
  try {
    const live = await invoke<LibrarySection[]>("knowledge_library", {});
    sources.set(sortSections(live));
    libraryMocked.set(false);
    libraryUnavailable.set(false);
    libraryNoService.set(false);
  } catch (e) {
    if (!tauriAvailable) {
      sources.set(sortSections(FIXTURE));
      libraryMocked.set(true);
      libraryUnavailable.set(false);
      libraryNoService.set(false);
      return;
    }
    sources.set([]);
    libraryMocked.set(false);
    // A daemon that is not running and a read that failed are different
    // sentences. The backend answers the first with a marker token rather than
    // prose, so this can tell them apart without matching on a message.
    //
    // The third case this branch used to carry - the command not existing at all
    // - is gone: `knowledge_library` is registered now, so the "not built yet"
    // sentence could never be reached again and a branch that cannot fire is a
    // claim nobody can check.
    const absent = isServiceAbsent(e);
    libraryNoService.set(absent);
    libraryUnavailable.set(!absent);
  }
}
