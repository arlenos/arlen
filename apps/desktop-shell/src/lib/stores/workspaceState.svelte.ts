import type { WorkspaceInfo } from "./workspaces.js";

/// Module-level reactive workspace state. Mutated from +layout.svelte's
/// store subscription, read directly by WorkspaceIndicator.svelte.
export const wsState = $state({
    list: [] as WorkspaceInfo[],
    activeId: null as string | null,
});
