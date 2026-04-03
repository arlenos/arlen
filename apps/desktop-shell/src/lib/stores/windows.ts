import { listen } from "@tauri-apps/api/event";
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
    listen<WindowInfo>("lunaris://toplevel-added", (event) => {
        windows.update((ws) => [...ws, event.payload]);
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
