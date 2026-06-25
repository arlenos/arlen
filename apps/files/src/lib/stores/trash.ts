/// The Trash view state + the trash command wrappers. Trash is a virtual view
/// (like search results), not a folder the browser navigates into: opening it
/// lists the home trash via `files_trash_list`, and the per-item Restore +
/// Empty actions call the matching backend commands and reload. The trash trio
/// (list / empty / restore) is built + tested in the FM core; this is its UI
/// data layer.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { overlay, closeOverlay } from "./overlay";

/// One trashed entry (mirrors the Rust `ops::TrashedItem`): the opaque token
/// the backend addresses it by, its recorded original path, and the deletion
/// date string.
export interface TrashedItem {
  trashed_name: string;
  original_path: string;
  deletion_date: string;
}

/// The current trash contents; null = not loaded yet, [] = loaded and empty.
export const trashItems = writable<TrashedItem[] | null>(null);

/// Load (or reload) the home trash contents into the store. A backend error
/// leaves an empty list rather than a stale view.
export async function loadTrash(): Promise<void> {
  try {
    trashItems.set(await invoke<TrashedItem[]>("files_trash_list"));
  } catch {
    trashItems.set([]);
  }
}

/// Open the Trash view (exclusive of other overlays) and load its contents.
export async function openTrash(): Promise<void> {
  overlay.set("trash");
  await loadTrash();
}

/// Close the Trash view, back to the listing.
export function closeTrash(): void {
  closeOverlay();
}

/// Restore one entry to its recorded original location (the backend reanchors
/// it to the FM root capability, rename-on-conflict), then reload the view.
export async function restoreTrashItem(item: TrashedItem): Promise<void> {
  try {
    await invoke("files_trash_restore", {
      trashedName: item.trashed_name,
      originalPath: item.original_path,
    });
  } finally {
    await loadTrash();
  }
}

/// Permanently empty the trash; returns the number of entries cleared, then
/// reloads (to the now-empty view).
export async function emptyTrash(): Promise<number> {
  try {
    return await invoke<number>("files_trash_empty");
  } finally {
    await loadTrash();
  }
}
