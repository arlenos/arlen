import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived, type Readable } from "svelte/store";
import { windows, type WindowInfo } from "./windows.js";

export interface WorkspaceInfo {
    id: string;
    /** ID of the workspace group (one group per output/monitor). */
    group_id: string;
    name: string;
    active: boolean;
}

/// Full workspace list across all outputs, sorted by compositor coordinates.
export const workspaces = writable<WorkspaceInfo[]>([]);

/// Workspaces belonging to the first (primary) group only.
/// On single-monitor setups this is all workspaces. On multi-monitor
/// setups this filters to the primary output's group.
export const primaryWorkspaces = derived(workspaces, ($ws) => {
    if ($ws.length === 0) return [];
    const primaryGroup = $ws[0].group_id;
    return $ws.filter((w) => w.group_id === primaryGroup);
});

/// The currently active workspace on the primary output, or null.
export const activeWorkspace = derived(primaryWorkspaces, ($ws) =>
    $ws.find((w) => w.active) ?? null
);

/// Registers the Tauri event listener for `lunaris://workspace-list`.
/// Must be called once from +layout.svelte onMount.
/// Counter incremented on every workspace-list event, exposed so components
/// can verify the store is updating.
export const wsUpdateCount = writable(0);

export function initWorkspaceListeners() {
    listen<WorkspaceInfo[]>("lunaris://workspace-list", ({ payload }) => {
        workspaces.set(payload);
        wsUpdateCount.update((n) => n + 1);
    });
}

/// Returns a derived store of all windows assigned to the given workspace.
export function windowsOnWorkspace(workspaceId: string): Readable<WindowInfo[]> {
    return derived(windows, ($windows) =>
        $windows.filter((w) => w.workspace_ids.includes(workspaceId))
    );
}

/// Sends a workspace activation request to the compositor.
export async function activateWorkspace(id: string): Promise<void> {
    await invoke("workspace_activate", { id });
}
