/// The search model (knowledge-app.md KA-R4, decision 2): structured + NAME
/// search now, honestly labelled; the semantic by-meaning layer arrives only
/// once `retrieval.rs` maturity is verified, as its own labelled group.
/// Refinement is guided facets (the SemFacet finding), never one-shot
/// NL-to-query. Saved searches are the query-as-folder bet: a query + its
/// facets kept as a runnable place.
///
/// Mock-vs-live: `knowledge_search` is the coder seam; under vite a local
/// index assembled from the one fixture story (projects, library, timeline)
/// stands in and `searchMocked` says so.
import { writable, derived, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// What kind of node a result is; drives the type tag and the type facet.
export type ResultType = "file" | "project" | "paper" | "mail" | "note" | "session";

/// One search hit, already display-shaped.
export interface SearchResult {
  id: string;
  type: ResultType;
  /// The emphasized name.
  title: string;
  /// The quiet context: the source app or bridge.
  sub: string;
  /// Unix seconds of the latest touch, when known.
  at?: number;
  /// The project it belongs to, when the graph knows one.
  project?: string;
}

/// The guided facets. null means "any".
export interface SearchFacets {
  type: ResultType | null;
  project: string | null;
  /// Days back, or null for any time.
  withinDays: number | null;
}

/// The live query text (the titlebar writes it).
export const query = writable("");
/// The active guided facets.
export const facets = writable<SearchFacets>({ type: null, project: null, withinDays: null });
/// True while results come from the FIXTURE index, not the graph.
export const searchMocked = writable(false);

const now = Math.floor(Date.now() / 1000);
const daysAgo = (d: number, h = 12): number => {
  const dd = new Date(now * 1000);
  dd.setHours(h, 0, 0, 0);
  return Math.floor(dd.getTime() / 1000) - d * 86400;
};

// The fixture index: the same story the timeline, projects and library tell,
// flattened into searchable rows. Dense on purpose - the graph's worth is
// cross-source context.
//
// i18n-foreign: the user's own things - a paper, a note, a project, a meeting.
// The graph will hand these over in whatever language they were written in, and
// a fixture standing in for them is not ours to translate either.
const INDEX: SearchResult[] = [
  { id: "r-1", type: "file", title: "compositor.toml", sub: "Text editor", at: daysAgo(2, 10), project: "Arlen OS" },
  { id: "r-2", type: "file", title: "design-system.md", sub: "Files", at: daysAgo(2, 11), project: "Arlen OS" },
  { id: "r-3", type: "file", title: "justfile", sub: "Terminal", at: daysAgo(2, 10), project: "Arlen OS" },
  { id: "r-4", type: "file", title: "chapter-3.md", sub: "Text editor", at: daysAgo(1, 11), project: "Thesis" },
  { id: "r-5", type: "file", title: "notes.md", sub: "Text editor", at: daysAgo(4, 18), project: "Thesis" },
  { id: "r-6", type: "file", title: "landing.fig", sub: "Files", at: daysAgo(0, 14), project: "Website redesign" },
  { id: "r-7", type: "file", title: "hero.css", sub: "Text editor", at: daysAgo(0, 16), project: "Website redesign" },
  { id: "r-8", type: "paper", title: "Attention Is All You Need", sub: "Zotero bridge", at: daysAgo(1, 13), project: "Thesis" },
  { id: "r-9", type: "note", title: "Deep work", sub: "Obsidian bridge", at: daysAgo(2, 9) },
  { id: "r-10", type: "mail", title: "Re: review notes", sub: "Thunderbird bridge", at: daysAgo(3, 8) },
  { id: "r-11", type: "paper", title: "The Rust Programming Language", sub: "Calibre bridge", at: daysAgo(3, 21) },
  { id: "r-12", type: "project", title: "Arlen OS", sub: "Detected from activity", at: daysAgo(2, 10) },
  { id: "r-13", type: "project", title: "Thesis", sub: "Detected from activity", at: daysAgo(1, 11) },
  { id: "r-14", type: "project", title: "Website redesign", sub: "Detected from activity", at: daysAgo(0, 16) },
  { id: "r-15", type: "session", title: "Website redesign, 2:10 pm to 4:40 pm", sub: "Session", at: daysAgo(0, 16), project: "Website redesign" },
  { id: "r-16", type: "session", title: "Arlen OS, 9:30 am to 12:15 pm", sub: "Session", at: daysAgo(2, 12), project: "Arlen OS" },
];

function matches(r: SearchResult, q: string, f: SearchFacets): boolean {
  if (f.type && r.type !== f.type) return false;
  if (f.project && r.project !== f.project) return false;
  if (f.withinDays !== null && (r.at ?? 0) < now - f.withinDays * 86400) return false;
  const needle = q.trim().toLowerCase();
  if (needle.length === 0) return true;
  return (
    r.title.toLowerCase().includes(needle) ||
    r.sub.toLowerCase().includes(needle) ||
    (r.project ?? "").toLowerCase().includes(needle)
  );
}

/// Live results. The seam is asked once per keystroke set; the fixture filter
/// answers synchronously via the derived fallback.
export const results = derived(
  [query, facets],
  ([$q, $f], set: (v: SearchResult[]) => void) => {
    if ($q.trim().length === 0 && !$f.type && !$f.project && $f.withinDays === null) {
      set([]);
      return;
    }
    invoke<SearchResult[]>("knowledge_search", { query: $q, facets: $f })
      .then((live) => {
        searchMocked.set(false);
        set(live);
      })
      .catch(() => {
        searchMocked.set(true);
        set(INDEX.filter((r) => matches(r, $q, $f)).sort((a, b) => (b.at ?? 0) - (a.at ?? 0)));
      });
  },
  [] as SearchResult[]
);

/// The project names the project facet offers (from the fixture; live this
/// is a typed read).
export function projectChoices(): string[] {
  return ["Arlen OS", "Thesis", "Website redesign"];
}

/// A saved search: the query-as-folder bet. Name + the exact query state.
export interface SavedSearch {
  id: string;
  name: string;
  query: string;
  facets: SearchFacets;
}

/// The searches this person saved, newest first, read from disk on mount.
///
/// It starts EMPTY. It used to open with four entries - "Touched by cargo
/// build", "Related to Thesis" - which rendered in every session including a
/// real one as searches the user had saved and had not. That is the fixture
/// defect without a catch to hide in: a hardcoded initial value.
export const savedSearches = writable<SavedSearch[]>([]);

/// The four former presets, kept for the design surface under vite only.
const DEV_PRESETS: SavedSearch[] = [
  { id: "s-1", name: "Touched by cargo build", query: "cargo", facets: { type: null, project: "Arlen OS", withinDays: null } },
  { id: "s-2", name: "Related to Thesis", query: "", facets: { type: null, project: "Thesis", withinDays: null } },
  { id: "s-3", name: "Opened this week", query: "", facets: { type: "file", project: null, withinDays: 7 } },
  { id: "s-4", name: "Papers I have not read", query: "", facets: { type: "paper", project: null, withinDays: null } },
];

/// True when the saved list could not be read, so an empty place can say which
/// kind of empty it is.
export const savedUnavailable = writable(false);

/// Load the saved searches. Call on mount.
export async function loadSavedSearches(): Promise<void> {
  try {
    savedSearches.set(await invoke<SavedSearch[]>("knowledge_searches"));
    savedUnavailable.set(false);
  } catch {
    if (import.meta.env.DEV) {
      savedSearches.set(DEV_PRESETS);
      return;
    }
    savedSearches.set([]);
    savedUnavailable.set(true);
  }
}

/// Keep the current query as a place. Live: `knowledge_search_save` (seam);
/// the optimistic add stands under vite.
export async function saveSearch(name: string): Promise<void> {
  const s: SavedSearch = {
    id: `s-${Math.random().toString(36).slice(2, 8)}`,
    name,
    query: get(query),
    facets: { ...get(facets) },
  };
  savedSearches.update((l) => [s, ...l]);
  try {
    // The command returns the list as written, so the place shows what is on
    // disk rather than what this window hoped was on disk.
    savedSearches.set(await invoke<SavedSearch[]>("knowledge_search_save", { search: s }));
  } catch {
    if (import.meta.env.DEV) return; // no backend under vite
    // A saved search that was not saved is gone at the next start, and the user
    // will look for it. Better it is not there now than not there later.
    savedSearches.update((l) => l.filter((x) => x.id !== s.id));
  }
}

/// Run a saved search: its state becomes the live state.
export function runSaved(s: SavedSearch): void {
  facets.set({ ...s.facets });
  query.set(s.query);
}

/// Clear everything (Esc in the titlebar field).
export function clearSearch(): void {
  query.set("");
  facets.set({ type: null, project: null, withinDays: null });
}
