/// The trash command wrappers. Trash is now a navigation LOCATION (the browser
/// lists it through `files_list_location`), so this is just the two actions its
/// entries expose: Restore one entry to its recorded original location, and
/// Empty the whole trash. Both are thin wrappers over the FM-core trash trio;
/// the caller refreshes the listing afterwards (the controller re-lists).

import { invoke } from "@tauri-apps/api/core";
import type { FileEntry } from "@arlen/ui-kit/components/browser";

/// Restore one trashed entry to its recorded original location (the backend
/// reanchors it to the FM root, rename-on-conflict). A Trash `FileEntry` carries
/// the address as `restore_token` (the trashed name) and the original path as
/// `full_path`; a non-trash entry (no token) is a no-op.
export async function restoreFromTrash(entry: FileEntry): Promise<void> {
  if (!entry.restore_token || !entry.full_path) return;
  await invoke("files_trash_restore", {
    trashedName: entry.restore_token,
    originalPath: entry.full_path,
  });
}

/// Permanently empty the trash; returns the number of entries cleared.
export async function emptyTrash(): Promise<number> {
  return invoke<number>("files_trash_empty");
}

/// Permanently delete one trashed entry, bypassing restore. A non-trash entry
/// (no `restore_token`) is a no-op; the caller re-lists afterwards.
export async function deletePermanently(entry: FileEntry): Promise<void> {
  if (!entry.restore_token) return;
  await invoke("files_trash_delete", { trashedName: entry.restore_token });
}
