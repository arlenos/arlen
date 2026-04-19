import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";

export interface WindowInfo {
    id: string;
    app_id: string;
    title: string;
    active: boolean;
    /** Workspace handle IDs this window belongs to (usually one). */
    workspace_ids: string[];
}

export const windows = writable<WindowInfo[]>([]);

export const activeWindow = derived(windows, ($windows) =>
    $windows.find((w) => w.active) ?? null
);

export const activeAppName = derived(activeWindow, ($active) => {
    if (!$active) return "";
    return $active.title || $active.app_id || "";
});

export function initWindowListeners() {
    // Prime the store with the backend's cached snapshot. Needed
    // because `toplevel-added` only fires for NEW toplevels after the
    // listener is installed — existing windows opened before a HMR
    // full-page reload never re-emit, leaving the store empty and any
    // window-card UI (WorkspaceIndicator overview) showing "Empty"
    // for every workspace until the next open-or-close event.
    invoke<WindowInfo[]>("get_windows")
        .then((initial) => {
            windows.update((current) =>
                current.length === 0 ? initial : current,
            );
        })
        .catch((e) => console.warn("get_windows failed", e));

    listen<WindowInfo>("lunaris://toplevel-added", (event) => {
        windows.update((ws) => {
            // Guard against duplicates when a prime + live-event arrive
            // for the same window near-simultaneously after a reload.
            if (ws.some((w) => w.id === event.payload.id)) return ws;
            return [...ws, event.payload];
        });
    });

    listen<WindowInfo>("lunaris://toplevel-changed", (event) => {
        windows.update((ws) =>
            ws.map((w) => (w.id === event.payload.id ? event.payload : w))
        );
    });

    listen<{ id: string }>("lunaris://toplevel-removed", (event) => {
        windows.update((ws) => ws.filter((w) => w.id !== event.payload.id));
    });
}
