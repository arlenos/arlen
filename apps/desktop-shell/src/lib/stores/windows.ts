import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";
import { makeDisposer } from "./_disposer.js";

export interface WindowInfo {
    id: string;
    app_id: string;
    title: string;
    active: boolean;
    /**
     * True when cosmic-toplevel-info reports `State::Minimized`.
     * Defaults to false so components that pre-existed this field
     * (e.g. older stores loaded from cache on HMR) don't get
     * undefined checks.
     */
    minimized?: boolean;
    /**
     * True when cosmic-toplevel-info reports `State::Fullscreen`.
     * Used by the context menu to label/toggle correctly.
     */
    fullscreen?: boolean;
    /** Workspace handle IDs this window belongs to (usually one). */
    workspace_ids: string[];
    /** Output connectors (`DP-1`, …) the window is currently visible on.
     *  Multi-output windows have more than one entry; absent during
     *  the brief startup race before xdg-output names arrive. */
    output_connectors?: string[];
}

export const windows = writable<WindowInfo[]>([]);

export const activeWindow = derived(windows, ($windows) =>
    $windows.find((w) => w.active) ?? null
);

/// Active window AS SEEN by a specific output. Returns the active
/// window when its `output_connectors` includes the given
/// connector, else null. Used by per-output GlobalMenuBar so each
/// bar only shows the menu of windows that physically live on
/// that monitor. When `connector` is null (registry race), falls
/// through to the legacy global `activeWindow` so the primary bar
/// stays populated during startup.
export function activeWindowForOutput(
    connector: string | null,
): typeof activeWindow {
    if (connector === null) return activeWindow;
    return derived(windows, ($windows) => {
        const a = $windows.find((w) => w.active);
        if (!a) return null;
        const outputs = a.output_connectors ?? [];
        if (outputs.length === 0) return a; // pre-resolution fallback
        return outputs.includes(connector) ? a : null;
    });
}

export const activeAppName = derived(activeWindow, ($active) => {
    if (!$active) return "";
    return $active.title || $active.app_id || "";
});

let started = false;
let teardown: (() => void) | null = null;

export function initWindowListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

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

    const pending: Array<Promise<UnlistenFn>> = [
        listen<WindowInfo>("lunaris://toplevel-added", (event) => {
            windows.update((ws) => {
                // Guard against duplicates when a prime + live-event arrive
                // for the same window near-simultaneously after a reload.
                if (ws.some((w) => w.id === event.payload.id)) return ws;
                return [...ws, event.payload];
            });
        }),
        listen<WindowInfo>("lunaris://toplevel-changed", (event) => {
            windows.update((ws) =>
                ws.map((w) => (w.id === event.payload.id ? event.payload : w))
            );
        }),
        listen<{ id: string }>("lunaris://toplevel-removed", (event) => {
            windows.update((ws) => ws.filter((w) => w.id !== event.payload.id));
        }),
    ];

    const disposer = makeDisposer(pending);
    teardown = () => {
        disposer();
        started = false;
        teardown = null;
    };
    return teardown;
}
