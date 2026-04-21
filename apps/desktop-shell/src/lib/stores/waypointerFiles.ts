/// File-search results from the `core.files` plugin.
///
/// Goes through the same generic manager bridge as `waypointerPower.ts`:
/// every keystroke calls `waypointer_search_plugin("core.files", q)`.
/// Execute flows back through `waypointer_execute` so the plugin owns
/// xdg-open dispatch and the "file no longer exists" error path.
///
/// Query modes understood by the plugin (parsed server-side):
///   `<substring>`        - substring match on basename or path
///   `f:<substring>`      - explicit "files only" prefix (same effect)
///   `project:<name>`     - files tagged as part of a project
///   `app:<id>`           - files accessed by a given app id

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// Mirrors the Rust `SearchResult` struct. Same as `PowerActionResult`
/// but typed here for clarity.
export interface FileResult {
    id: string;
    title: string;
    description: string | null;
    icon: string | null;
    relevance: number;
    action: unknown;
    plugin_id: string;
}

const _results = writable<FileResult[]>([]);
export const fileResults: Readable<FileResult[]> = {
    subscribe: _results.subscribe,
};

/// Fetch fresh results for the given query. Empty query -> empty
/// store. Failure is silent — a missing Knowledge daemon or a query
/// timeout should not surface as an error in the Waypointer.
export async function updateFileResults(query: string): Promise<void> {
    if (!query.trim()) {
        _results.set([]);
        return;
    }
    try {
        const r = await invoke<FileResult[]>("waypointer_search_plugin", {
            pluginId: "core.files",
            query,
        });
        _results.set(r);
    } catch (e) {
        console.warn("[waypointer] file search failed:", e);
        _results.set([]);
    }
}

export function clearFileResults(): void {
    _results.set([]);
}

/// Open a file via the plugin's `execute()` path. Triggers xdg-open on
/// the file, or surfaces an error if the path no longer exists. Errors
/// are swallowed after logging — the Waypointer closes afterwards
/// anyway and the user can retry.
export async function openFileResult(result: FileResult): Promise<void> {
    try {
        await invoke("waypointer_execute", { result });
    } catch (e) {
        console.warn("[waypointer] file open failed:", e);
    }
}
