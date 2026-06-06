/// Clipboard-history results from the `core.clipboard` plugin.
///
/// Privacy design: opt-in via `~/.config/lunaris/shell.toml`
/// `[clipboard] enabled = true`. The backend refuses to even spawn
/// the `wl-paste` watcher until that flag is set, so this store stays
/// empty and the Waypointer section is hidden by default.
///
/// Two paths to the backend:
///
/// 1. `updateClipboardResults(q)` -> `waypointer_search_plugin`:
///    substring filter, used on every keystroke. Goes through the
///    generic plugin bridge, same as Power and Files.
/// 2. `deleteClipboardEntry(id)` / `clearClipboard()` /
///    `copyClipboardEntry(id)` -> dedicated Tauri commands on
///    `clipboard_history`: direct manipulation of the ring buffer and
///    paste-back via `wl-copy`.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

export interface ClipboardResult {
    id: string;
    title: string;
    description: string | null;
    icon: string | null;
    relevance: number;
    action: unknown;
    plugin_id: string;
}

const _results = writable<ClipboardResult[]>([]);
export const clipboardResults: Readable<ClipboardResult[]> = {
    subscribe: _results.subscribe,
};

/// Cached opt-in flag. We call `clipboard_is_enabled` once on init
/// and skip further updates entirely when disabled. Saves a backend
/// round-trip per keystroke when the feature is off.
const _enabled = writable<boolean>(false);
export const clipboardEnabled: Readable<boolean> = {
    subscribe: _enabled.subscribe,
};

/// Read the enabled flag from the backend. Call once on shell startup
/// (or on Waypointer open) to prime the store; lazy-loading on first
/// keystroke is fine too. Safe to call repeatedly — the backend read
/// is a single atomic load.
export async function refreshClipboardEnabled(): Promise<void> {
    try {
        const e = await invoke<boolean>("clipboard_is_enabled");
        _enabled.set(e);
    } catch {
        _enabled.set(false);
    }
}

/// Fetch fresh results for the given query. Empty query -> empty
/// store (the Waypointer hides the Clipboard section on length === 0).
/// No-op when the feature is disabled.
export async function updateClipboardResults(query: string): Promise<void> {
    if (!query.trim()) {
        _results.set([]);
        return;
    }
    try {
        const r = await invoke<ClipboardResult[]>("waypointer_search_plugin", {
            pluginId: "core.clipboard",
            query,
        });
        _results.set(r);
    } catch (e) {
        console.warn("[waypointer] clipboard search failed:", e);
        _results.set([]);
    }
}

export function clearClipboardResults(): void {
    _results.set([]);
}

/// Parse the plugin's id field (format: "clip-<numeric id>") back to
/// the numeric id the backend expects. Kept colocated with the store
/// so every caller uses the same assumption.
function entryIdOf(result: ClipboardResult): number | null {
    const match = result.id.match(/^clip-(\d+)$/);
    return match ? Number(match[1]) : null;
}

/// Paste an entry back into the system clipboard via `wl-copy`. Goes
/// directly to the dedicated Tauri command (not through
/// `waypointer_execute`) so the UI can call this without owning a
/// full `SearchResult`.
export async function copyClipboardEntry(result: ClipboardResult): Promise<void> {
    const id = entryIdOf(result);
    if (id === null) {
        console.warn("[waypointer] clipboard id parse failed:", result.id);
        return;
    }
    try {
        await invoke("clipboard_copy_entry", { id });
    } catch (e) {
        console.warn("[waypointer] clipboard copy failed:", e);
    }
}

/// Remove a single entry from the ring buffer. Backend refreshes
/// affected consumers via `lunaris://clipboard-changed`.
export async function deleteClipboardEntry(result: ClipboardResult): Promise<void> {
    const id = entryIdOf(result);
    if (id === null) return;
    try {
        await invoke("clipboard_delete_entry", { id });
        // Optimistic: drop it from the current result set immediately.
        _results.update((arr) => arr.filter((r) => r.id !== result.id));
    } catch (e) {
        console.warn("[waypointer] clipboard delete failed:", e);
    }
}

/// Drop the entire ring buffer.
export async function clearAllClipboard(): Promise<void> {
    try {
        await invoke("clipboard_clear_all");
        _results.set([]);
    } catch (e) {
        console.warn("[waypointer] clipboard clear-all failed:", e);
    }
}
