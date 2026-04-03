import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived, type Readable } from "svelte/store";
import { windows, type WindowInfo } from "./windows.js";

export interface WorkspaceInfo {
    id: string;
    name: string;
    active: boolean;
}

/// Full workspace list, sorted by compositor coordinates.
export const workspaces = writable<WorkspaceInfo[]>([]);

/// The currently active workspace, or null before first update.
export const activeWorkspace = derived(workspaces, ($ws) =>
    $ws.find((w) => w.active) ?? null
);

/// Registers the Tauri event listener for `lunaris://workspace-list`.
/// Must be called once from +layout.svelte onMount.
export function initWorkspaceListeners() {
    listen<WorkspaceInfo[]>("lunaris://workspace-list", ({ payload }) => {
        workspaces.set(payload);
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
