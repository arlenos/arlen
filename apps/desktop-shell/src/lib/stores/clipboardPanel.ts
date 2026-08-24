/// The dedicated clipboard-history panel's state (clipboard-api.md). Distinct
/// from `waypointerClipboard.ts` on purpose: the Waypointer filters per
/// keystroke through the plugin bridge, while a panel takes ONE snapshot on
/// open - `clipboard_get_entries`, the command written for exactly this and
/// called by nothing until now - and filters locally.
///
/// What the spec fixes and the panel must not soften: text only, thirty
/// entries, nothing persists - the ring buffer dies with the shell (FA12, a
/// threat-model rule, not a stopgap), so an empty panel after login is the
/// designed state and gets said as one. Sensitive entries are filtered at
/// write time and never reach a snapshot.

import { writable } from "svelte/store";
import { invoke, isTauri } from "@tauri-apps/api/core";

/// One history entry as `clipboard_get_entries` serialises it (camelCase).
export interface ClipboardPanelEntry {
  id: number;
  content: string;
  timestampMs: number;
  sourceAppId: string;
  mime: string;
}

/// The snapshot, newest first, or null before the read settles.
export const clipEntries = writable<ClipboardPanelEntry[] | null>(null);
/// The opt-in flag; null before it was asked.
export const clipEnabled = writable<boolean | null>(null);
/// True while the list is the fixture (design work under vite).
export const clipMocked = writable(false);
/// True when a real session could not read the history - not the same as
/// empty, and the panel says a different sentence for each.
export const clipUnavailable = writable(false);
/// True when the last copy-back did not reach the clipboard.
export const clipCopyFailed = writable(false);

const now = Date.now();

const FIXTURE: ClipboardPanelEntry[] = [
  { id: 6, content: "arlen-shell: the clipboard panel takes one snapshot on open", timestampMs: now - 30_000, sourceAppId: "org.arlen.text-editor", mime: "text/plain" },
  { id: 5, content: "https://arlen.dev/docs/clipboard-api", timestampMs: now - 4 * 60_000, sourceAppId: "org.arlen.files", mime: "text/plain" },
  { id: 4, content: "cargo build --manifest-path apps/desktop-shell/src-tauri/Cargo.toml", timestampMs: now - 9 * 60_000, sourceAppId: "org.arlen.terminal", mime: "text/plain" },
  { id: 3, content: "Dear Ms Winter,\n\nthank you for the quick reply. The revised draft is attached;\nthe second section now carries the numbers we discussed.", timestampMs: now - 21 * 60_000, sourceAppId: "org.arlen.mail", mime: "text/plain" },
  { id: 2, content: "1873 Hofsteigstraße, Wolfurt", timestampMs: now - 48 * 60_000, sourceAppId: "org.arlen.files", mime: "text/plain" },
];

/// Load the opt-in flag and the snapshot. Live: `clipboard_is_enabled` +
/// `clipboard_get_entries`; fixture under vite.
export async function loadClipboardPanel(): Promise<void> {
  clipCopyFailed.set(false);
  try {
    const enabled = await invoke<boolean>("clipboard_is_enabled");
    clipEnabled.set(enabled);
    if (!enabled) {
      clipEntries.set([]);
      clipMocked.set(false);
      clipUnavailable.set(false);
      return;
    }
    const entries = await invoke<ClipboardPanelEntry[]>("clipboard_get_entries");
    clipEntries.set([...entries].sort((a, b) => b.timestampMs - a.timestampMs));
    clipMocked.set(false);
    clipUnavailable.set(false);
  } catch {
    if (isTauri()) {
      // A real session whose read failed. Null is "unknown", not "empty":
      // the panel says it cannot show the history rather than claiming a
      // clean slate.
      clipEntries.set(null);
      clipEnabled.set(null);
      clipMocked.set(false);
      clipUnavailable.set(true);
      return;
    }
    clipEnabled.set(true);
    clipEntries.set(FIXTURE.map((e) => ({ ...e })));
    clipMocked.set(true);
    clipUnavailable.set(false);
  }
}

/// Copy one entry back to the system clipboard. Live: `clipboard_copy_entry`
/// (wl-copy write-back). Returns whether the caller may close the panel - a
/// failed copy keeps it open with the sentence, because closing reads as done.
export async function copyPanelEntry(id: number): Promise<boolean> {
  clipCopyFailed.set(false);
  try {
    await invoke("clipboard_copy_entry", { id });
    return true;
  } catch {
    if (!isTauri()) return true; // no host, nothing to copy into - the mock closes
    clipCopyFailed.set(true);
    return false;
  }
}

/// Remove one entry. Optimistic; a live failure reloads the snapshot so the
/// list shows what the ring buffer really holds.
export async function deletePanelEntry(id: number): Promise<void> {
  clipEntries.update((l) => (l ? l.filter((e) => e.id !== id) : l));
  try {
    await invoke("clipboard_delete_entry", { id });
  } catch {
    if (isTauri()) await loadClipboardPanel();
  }
}

/// Drop the whole history. The backend deletes without asking - "the UI is
/// expected to confirm before invoking" (its own comment) - so callers go
/// through the panel's click-again confirm first.
export async function clearPanel(): Promise<void> {
  clipEntries.set([]);
  try {
    await invoke("clipboard_clear_all");
  } catch {
    if (isTauri()) await loadClipboardPanel();
  }
}
