import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";

export interface WindowInfo {
    id: string;
    app_id: string;
    title: string;
    active: boolean;
    workspace_ids: string[];
}

/// Filtered window results for Waypointer search.
export const windowResults = writable<WindowInfo[]>([]);

/// Filters open windows by query. Fetches the current window list from Rust
/// (separate WebView cannot share the windows.ts store with the main window).
export function updateWindowResults(query: string) {
    if (!query || query.length < 2) {
        windowResults.set([]);
        return;
    }
    const lower = query.toLowerCase();
    invoke<WindowInfo[]>("get_windows")
        .then((wins) => {
            const matched = wins
                .filter((w) =>
                    w.title.toLowerCase().includes(lower) ||
                    w.app_id.toLowerCase().includes(lower)
                )
                .slice(0, 5);
            windowResults.set(matched);
        })
        .catch(() => { windowResults.set([]); });
}

/// Clears window results.
export function clearWindowResults() {
    windowResults.set([]);
}

/// Activates (focuses) a window by its identifier.
export function activateWindow(id: string) {
    invoke("activate_window", { id });
}
