// Value + type imports split — inline mixed form trips Tailwind's
// Vite plugin CSS parser and cascades into bogus "Invalid declaration"
// errors on subsequent value-only imports in the same pipeline run.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";
import type { Readable } from "svelte/store";
import { windows } from "./windows.js";
import type { WindowInfo } from "./windows.js";
import { makeDisposer } from "./_disposer.js";

export interface WorkspaceInfo {
    id: string;
    /** ID of the workspace group (one group per output/monitor). */
    group_id: string;
    name: string;
    active: boolean;
    /** Output connectors this workspace's group spans (resolved from
     *  cosmic workspace_groups + xdg-output names). Empty when the
     *  compositor hasn't reported the connector mapping yet. */
    output_connectors: string[];
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

let started = false;
let teardown: (() => void) | null = null;

export function initWorkspaceListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

    // Prime the store synchronously with the backend's cached
    // snapshot. Needed because the compositor only emits
    // `lunaris://workspace-list` on state changes; after a Vite
    // HMR full-page reload the Svelte store is reset to `[]` and
    // no event ever re-populates it, which left the
    // WorkspaceIndicator hidden (`{#if length > 0}` guard) until
    // the user triggered a compositor event manually.
    invoke<WorkspaceInfo[]>("get_workspaces")
        .then((initial) => {
            // Don't clobber if a live event already arrived between
            // the invoke call and its resolution.
            workspaces.update((current) =>
                current.length === 0 ? initial : current,
            );
            if (initial.length > 0) wsUpdateCount.update((n) => n + 1);
        })
        .catch((e) => console.warn("get_workspaces failed", e));

    const pending: Array<Promise<UnlistenFn>> = [
        listen<WorkspaceInfo[]>("lunaris://workspace-list", ({ payload }) => {
            workspaces.set(payload);
            wsUpdateCount.update((n) => n + 1);
        }),
    ];

    const disposer = makeDisposer(pending);
    teardown = () => { disposer(); started = false; teardown = null; };
    return teardown;
}

/// Returns a derived store of all windows assigned to the given workspace.
export function windowsOnWorkspace(workspaceId: string): Readable<WindowInfo[]> {
    return derived(windows, ($windows) =>
        $windows.filter((w) => w.workspace_ids.includes(workspaceId))
    );
}

/// Workspaces belonging to the named output. The per-output
/// TopBar's `WorkspaceIndicator` filters its strip on this so each
/// monitor's bar only shows that monitor's workspaces. When the
/// connector is `null` (registry not populated yet) we fall back
/// to the legacy primary-only filter so the primary bar's first
/// paint still has a populated strip — the secondary bars stay
/// empty until the compositor reports their connector.
export function workspacesByOutput(
    connector: string | null,
): Readable<WorkspaceInfo[]> {
    return derived(workspaces, ($ws) => {
        if ($ws.length === 0) return [];
        if (connector === null) {
            const primary = $ws[0].group_id;
            return $ws.filter((w) => w.group_id === primary);
        }
        return $ws.filter((w) => w.output_connectors.includes(connector));
    });
}

/// Sends a workspace activation request to the compositor.
export async function activateWorkspace(id: string): Promise<void> {
    await invoke("workspace_activate", { id });
}
