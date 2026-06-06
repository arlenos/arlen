/// Minimized-windows store.
///
/// Source of truth: the cosmic-toplevel-info protocol that populates
/// `windows.ts`. Each `WindowInfo` now carries a `minimized: boolean`
/// flag, so we derive the minimized list directly from that store
/// instead of maintaining a second subscription. Keeps both sides
/// coherent automatically: if a window is closed or its state flips,
/// the derived store reflects it on the next `windows` tick without
/// any event plumbing here.
///
/// The Rust side also exposes a `get_minimized_windows` Tauri command
/// that returns the same data with pre-resolved icon paths. The
/// frontend uses that path on popover/indicator mount to skip the
/// per-icon `resolve_app_icon` round-trip that the derived store
/// would otherwise incur.

import { derived, writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { windows, type WindowInfo } from "./windows.js";

/// Shape returned by the `get_minimized_windows` Tauri command.
/// camelCase matches the Rust `#[serde(rename_all = "camelCase")]`.
export interface MinimizedWindow {
    windowId: string;
    appId: string;
    title: string;
    workspaceId: string;
    iconPath: string | null;
}

/// Derived from the full window list: simple filter + projection.
/// No workspace filtering here — callers that need per-workspace
/// grouping use `minimizedByWorkspace` below.
export const minimizedWindows: Readable<MinimizedWindow[]> = derived(
    windows,
    ($windows) =>
        $windows
            .filter((w: WindowInfo) => w.minimized)
            .map((w: WindowInfo) => ({
                windowId: w.id,
                appId: w.app_id,
                title: w.title,
                workspaceId: w.workspace_ids[0] ?? "",
                iconPath: null, // resolved lazily on first render
            })),
);

/// Per-workspace grouping. `Map<workspaceId, MinimizedWindow[]>`.
/// Workspace id is derived from the window's first workspace id;
/// sticky windows (empty id) land in the `""` bucket. The indicator
/// ignores that bucket — sticky-window minimize is edge case for
/// Phase 5.
export const minimizedByWorkspace: Readable<Map<string, MinimizedWindow[]>> =
    derived(minimizedWindows, ($list) => {
        const map = new Map<string, MinimizedWindow[]>();
        for (const m of $list) {
            const bucket = map.get(m.workspaceId);
            if (bucket) {
                bucket.push(m);
            } else {
                map.set(m.workspaceId, [m]);
            }
        }
        return map;
    });

/// Extra per-app-id icon cache. Filled by `loadMinimizedWindows()`
/// which invokes the Rust command (that does the icon resolution
/// eagerly). The WorkspaceIndicator then reads from this map to paint
/// icons without one `resolve_app_icon` invoke per render.
const _iconCache = writable<Record<string, string | null>>({});
export const minimizedIconCache: Readable<Record<string, string | null>> = {
    subscribe: _iconCache.subscribe,
};

/// Prime the icon cache from the Rust side. Called once on mount.
/// Safe to call repeatedly — the Rust command is cheap and the cache
/// is merged, not replaced. Returns early if the Wayland state is
/// not yet bound (happens during a HMR reload before the
/// `wayland_client` start has completed).
export async function loadMinimizedWindows(): Promise<void> {
    try {
        const list = await invoke<MinimizedWindow[]>("get_minimized_windows");
        _iconCache.update((current) => {
            const next = { ...current };
            for (const m of list) {
                if (m.iconPath) {
                    next[m.appId] = m.iconPath;
                }
            }
            return next;
        });
    } catch (e) {
        console.warn("[minimizedWindows] load failed:", e);
    }
}

/// Restore a minimized window on its current workspace.
export async function restoreWindow(windowId: string): Promise<void> {
    try {
        await invoke("restore_window", { windowId });
    } catch (e) {
        console.warn("[minimizedWindows] restore failed:", e);
    }
}

/// Move-and-restore: move the minimized window to `workspaceId`,
/// un-minimize, and focus. The backend also switches the active
/// workspace so the user sees the restored window immediately.
export async function restoreWindowToWorkspace(
    windowId: string,
    workspaceId: string,
): Promise<void> {
    try {
        await invoke("restore_window_to_workspace", { windowId, workspaceId });
    } catch (e) {
        console.warn("[minimizedWindows] restore-to-workspace failed:", e);
    }
}

/// Convenience re-export for components that want the count of
/// minimized windows on a given workspace (for e.g. badges in the
/// Workspace Overlay).
export function minimizedCountFor(
    byWorkspace: Map<string, MinimizedWindow[]>,
    workspaceId: string,
): number {
    return byWorkspace.get(workspaceId)?.length ?? 0;
}

/// Politely ask the window's app to close. The compositor sends the
/// standard cosmic-toplevel close request — apps with unsaved work
/// can prompt the user. If the close succeeds the toplevel-removed
/// event propagates through `windows.ts` and the icon disappears.
export async function closeMinimizedWindow(windowId: string): Promise<void> {
    try {
        await invoke("close_minimized_window", { windowId });
    } catch (e) {
        console.warn("[minimizedWindows] close failed:", e);
    }
}

/// Minimize a currently-visible window. Triggered from the overlay
/// when the user drags an active window card into a workspace's
/// Minimized area. The backend sends `set_minimized(true)` and the
/// resulting state change propagates back via cosmic-toplevel-info.
export async function minimizeWindow(windowId: string): Promise<void> {
    try {
        await invoke("minimize_window", { windowId });
    } catch (e) {
        console.warn("[minimizedWindows] minimize failed:", e);
    }
}
