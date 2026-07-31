/// The Library model (knowledge-app.md §3b, decision 7): the bridged
/// knowledge content - papers, books, notes, mail - browsed BY SOURCE, each
/// section carrying its origin tag (the same origin a per-source revoke
/// severs). Live: `knowledge_library` reads the origin-tagged bridge types
/// (a coder seam); under vite the fixture stands in, telling the same story
/// as the timeline and search, and `libraryMocked` says so.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One bridged item, display-shaped.
export interface LibraryEntry {
  id: string;
  /// The emphasized title.
  title: string;
  /// The quiet context: author, sender or folder.
  sub: string;
  /// Unix seconds of the item's own moment (published, received, edited).
  at: number;
}

/// One source section: the content class plus the bridge it came from.
export interface LibrarySource {
  key: "papers" | "books" | "notes" | "mail";
  /// The origin tag, named plainly ("Zotero bridge").
  bridge: string;
  entries: LibraryEntry[];
}

/// True while the sections are the FIXTURE, not the graph.
export const libraryMocked = writable(false);
/// The loaded sections, or null before the read settles.
export const sources = writable<LibrarySource[] | null>(null);

const now = Math.floor(Date.now() / 1000);
const daysAgo = (d: number, h = 12): number => {
  const dd = new Date(now * 1000);
  dd.setHours(h, 0, 0, 0);
  return Math.floor(dd.getTime() / 1000) - d * 86400;
};

const FIXTURE: LibrarySource[] = [
  {
    key: "papers",
    bridge: "Zotero bridge",
    entries: [
      { id: "p-1", title: "Attention Is All You Need", sub: "Vaswani et al, 2017", at: daysAgo(1, 13) },
      { id: "p-2", title: "Deep Residual Learning for Image Recognition", sub: "He et al, 2015", at: daysAgo(1, 13) },
      { id: "p-3", title: "Scaling Laws for Neural Language Models", sub: "Kaplan et al, 2020", at: daysAgo(6, 9) },
    ],
  },
  {
    key: "books",
    bridge: "Calibre bridge",
    entries: [
      { id: "b-1", title: "The Rust Programming Language", sub: "Klabnik and Nichols", at: daysAgo(3, 21) },
      { id: "b-2", title: "Designing Data-Intensive Applications", sub: "Kleppmann", at: daysAgo(20, 19) },
    ],
  },
  {
    key: "notes",
    bridge: "Obsidian bridge",
    entries: [
      { id: "n-1", title: "Deep work", sub: "Reading notes", at: daysAgo(2, 9) },
      { id: "n-2", title: "Thesis outline", sub: "Thesis", at: daysAgo(5, 16) },
    ],
  },
  {
    key: "mail",
    bridge: "Thunderbird bridge",
    entries: [
      { id: "m-1", title: "Re: review notes", sub: "alex@example.com", at: daysAgo(3, 8) },
      { id: "m-2", title: "Conference registration confirmed", sub: "orga@conf.example.org", at: daysAgo(9, 11) },
    ],
  },
];

/// Load the sections. Live: `knowledge_library` (seam); fixture under vite.
export async function loadLibrary(): Promise<void> {
  try {
    const live = await invoke<LibrarySource[]>("knowledge_library", {});
    sources.set(live);
    libraryMocked.set(false);
  } catch {
    sources.set(FIXTURE);
    libraryMocked.set(true);
  }
}
