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
///
/// When Focus Mode is active and the user types a bare substring (no
/// explicit prefix), this store auto-prepends `project:<focused-name>`
/// before sending the query to the plugin. An explicit `project:`,
/// `f:`, or `app:` prefix is treated as a deliberate override and
/// bypasses the auto-scope. See `applyFocusScope` below.

import { writable, type Readable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { focusState } from "./projects.js";

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

/// Auto-scope the query to the focused project when Focus Mode is on.
///
/// Per docs/architecture/project-system.md §Focus Mode → State
/// Changes table, file search should narrow to the active project.
/// We honour a manual `f:`, `app:`, or `project:` prefix as an
/// explicit override so the user can still search globally or against
/// a different project without leaving Focus.
function applyFocusScope(query: string): string {
    const trimmed = query.trim();
    if (
        trimmed.startsWith("f:") ||
        trimmed.startsWith("app:") ||
        trimmed.startsWith("project:")
    ) {
        return query;
    }
    const focus = get(focusState);
    if (!focus.projectName) {
        return query;
    }
    return `project:${focus.projectName} ${query}`;
}

/// Fetch fresh results for the given query. Empty query -> empty
/// store. Failure is silent — a missing Knowledge daemon or a query
/// timeout should not surface as an error in the Waypointer.
export async function updateFileResults(query: string): Promise<void> {
    if (!query.trim()) {
        _results.set([]);
        return;
    }
    const scoped = applyFocusScope(query);
    try {
        const r = await invoke<FileResult[]>("waypointer_search_plugin", {
            pluginId: "core.files",
            query: scoped,
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
