/// The Recent view state + loader. Recent is a virtual view (like Trash, an
/// overlay over the listing) of the KG's most-recently-accessed files; opening
/// a row opens that file with the system handler. Backed by `files_recent`
/// (the File nodes ordered by `last_accessed`).

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { overlay, closeOverlay } from "./overlay";

/// One recent file (mirrors the Rust `RecentFile`): absolute path (the File
/// node id), basename, and last-accessed time in microseconds since the epoch.
export interface RecentFile {
  path: string;
  name: string;
  accessed: number;
}

/// The recent list; null = not loaded yet, [] = loaded and empty.
export const recentItems = writable<RecentFile[] | null>(null);

/// Load (or reload) the recent files. A backend or graph error leaves an empty
/// list rather than a stale view (the KG may simply be unreachable).
export async function loadRecent(): Promise<void> {
  try {
    recentItems.set(await invoke<RecentFile[]>("files_recent"));
  } catch {
    recentItems.set([]);
  }
}

/// Open the Recent view (exclusive of other overlays) and load it.
export async function openRecent(): Promise<void> {
  overlay.set("recent");
  await loadRecent();
}

/// Close the Recent view, back to the listing.
export function closeRecent(): void {
  closeOverlay();
}
