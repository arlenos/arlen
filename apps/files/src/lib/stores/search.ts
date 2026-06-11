/// Search state: one bar over the current location with two facet
/// filters (type, time). The backend does the bounded name walk
/// (`files_search`); the facets narrow client-side over the hits.
/// Saving a search as a sidebar place needs a write command the
/// contract does not have yet (flagged); until it lands, saved
/// searches live for the session.

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { FileEntry } from "@arlen/ui-kit/components/browser";

export interface SearchHit {
  rel_path: string;
  entry: FileEntry;
  matched: "name" | "content" | "both";
}

interface SearchOutcome {
  hits: SearchHit[];
  truncated: boolean;
  content_budget_exhausted: boolean;
  examined_capped: boolean;
}

export type TypeFacet =
  | "any"
  | "folder"
  | "document"
  | "image"
  | "audio"
  | "video"
  | "archive"
  | "code";
export type TimeFacet = "any" | "day" | "week" | "month";

export const searchOpen = writable(false);
export const searchQuery = writable("");
export const searchType = writable<TypeFacet>("any");
export const searchTime = writable<TimeFacet>("any");
/// null = no search ran yet; [] = ran and found nothing.
export const searchResults = writable<SearchHit[] | null>(null);
export const searchTruncated = writable(false);

const EXT_FACETS: Record<string, TypeFacet> = {};
const put = (facet: TypeFacet, exts: string[]) => {
  for (const e of exts) EXT_FACETS[e] = facet;
};
put("document", ["md", "txt", "pdf", "rtf", "odt", "doc", "docx", "ods", "tex"]);
put("image", ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif"]);
put("audio", ["mp3", "flac", "ogg", "wav", "opus", "m4a"]);
put("video", ["mp4", "mkv", "webm", "avi", "mov"]);
put("archive", ["zip", "tar", "gz", "xz", "zst", "7z", "rar", "iso", "deb"]);
put("code", ["rs", "ts", "js", "svelte", "py", "c", "h", "go", "sh", "css", "html", "json", "toml", "yml", "yaml"]);

function facetOf(entry: FileEntry): TypeFacet {
  if (entry.kind === "directory") return "folder";
  const i = entry.name.lastIndexOf(".");
  const ext = i > 0 ? entry.name.slice(i + 1).toLowerCase() : "";
  return EXT_FACETS[ext] ?? "any";
}

function passesFacets(hit: SearchHit, now: number): boolean {
  const type = get(searchType);
  if (type !== "any" && facetOf(hit.entry) !== type) return false;
  const time = get(searchTime);
  if (time !== "any") {
    const cutoff =
      now - (time === "day" ? 86400 : time === "week" ? 7 * 86400 : 30 * 86400);
    if ((hit.entry.modified_unix ?? 0) < cutoff) return false;
  }
  return true;
}

let debounce: ReturnType<typeof setTimeout> | null = null;

export async function runSearch(path: string): Promise<void> {
  const query = get(searchQuery).trim();
  if (query.length === 0) {
    searchResults.set(null);
    searchTruncated.set(false);
    return;
  }
  try {
    const outcome = await invoke<SearchOutcome>("files_search", { path, query });
    const now = Date.now() / 1000;
    searchResults.set(outcome.hits.filter((h) => passesFacets(h, now)));
    searchTruncated.set(outcome.truncated);
  } catch {
    searchResults.set([]);
    searchTruncated.set(false);
  }
}

export function queueSearch(path: string): void {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    void runSearch(path);
  }, 200);
}

export function closeSearch(): void {
  searchOpen.set(false);
  searchQuery.set("");
  searchResults.set(null);
  searchTruncated.set(false);
}
