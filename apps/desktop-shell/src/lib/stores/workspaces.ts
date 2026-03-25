import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";

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

/// Sends a workspace activation request to the compositor.
export async function activateWorkspace(id: string): Promise<void> {
    await invoke("workspace_activate", { id });
}
