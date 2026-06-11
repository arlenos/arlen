<script lang="ts">
  /// Context-menu content shared by the active-window cards and the
  /// minimized-window cards. Branches three ways based on the
  /// current selection:
  /// - Multi-select: Close All / Minimize All / Restore All /
  ///   Move All to / (optional) Tile Side by Side.
  /// - Single active: Close / Minimize / Move to / Tile Left / Tile
  ///   Right / Fullscreen.
  /// - Single minimized: Restore / Close / Move to.
  /// Reads `$selectedWindowIds` and `$windows` directly — the menu
  /// must reflect the live selection at open time.

  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu/index.js";
  import { windows } from "$lib/stores/windows.js";
  import type { WindowInfo } from "$lib/stores/windows.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { selectedWindowIds } from "$lib/stores/overlaySelection.js";
  import {
    restoreWindow,
    minimizeWindow,
    closeMinimizedWindow,
    restoreWindowToWorkspace,
  } from "$lib/stores/minimizedWindows.js";
  import {
    closeWindowAction,
    fullscreenWindowAction,
    tileWindowAction,
    moveWindowToWorkspaceAction,
    closeAllSelected,
    minimizeAllSelected,
    restoreAllSelected,
    moveAllSelectedToWorkspace,
    tileSideBySide,
  } from "$lib/workspace/actions.js";

  let {
    windowId,
    isMinimized,
    workspaces,
    hideOverlay,
  }: {
    windowId: string;
    isMinimized: boolean;
    /// The per-output workspace view — source of the "Move to"
    /// submenu targets.
    workspaces: WorkspaceInfo[];
    /// State-only overlay close. The restore / tile paths use it —
    /// they never collapse the input region themselves (the
    /// compositor refocuses the restored / tiled windows on its
    /// own).
    hideOverlay: () => void;
  } = $props();

  const sel = $derived(Array.from($selectedWindowIds));
  const multi = $derived(sel.length > 1 && sel.includes(windowId));
  const win = $derived($windows.find((w) => w.id === windowId));
  const currentWs = $derived(win?.workspace_ids[0] ?? "");
  const moveTargets = $derived(
    workspaces.filter((ws) => ws.id !== currentWs),
  );

  function restoreAllSelectedAndClose(): void {
    restoreAllSelected();
    hideOverlay();
  }

  function tileSideBySideAndClose(ids: [string, string]): void {
    tileSideBySide(ids);
    hideOverlay();
  }
</script>

{#if multi}
  {@const selWindows = sel
    .map((id) => $windows.find((w) => w.id === id))
    .filter((w): w is WindowInfo => Boolean(w))}
  {@const anyActive = selWindows.some((w) => !w.minimized)}
  {@const anyMinimized = selWindows.some((w) => w.minimized)}
  {@const twoActive = selWindows.length === 2 && selWindows.every((w) => !w.minimized)}

  <ContextMenu.Item onclick={closeAllSelected}>
    Close All ({sel.length})
  </ContextMenu.Item>
  {#if anyActive}
    <ContextMenu.Item onclick={minimizeAllSelected}>Minimize All</ContextMenu.Item>
  {/if}
  {#if anyMinimized}
    <ContextMenu.Item onclick={restoreAllSelectedAndClose}>Restore All</ContextMenu.Item>
  {/if}
  {#if moveTargets.length > 0}
    <ContextMenu.Separator />
    <ContextMenu.Sub>
      <ContextMenu.SubTrigger>Move All to</ContextMenu.SubTrigger>
      <ContextMenu.Portal>
        <ContextMenu.SubContent class="shell-popover">
          {#each moveTargets as ws, i (ws.id)}
            <ContextMenu.Item onclick={() => moveAllSelectedToWorkspace(ws.id)}>
              {ws.name || `Workspace ${i + 1}`}
            </ContextMenu.Item>
          {/each}
        </ContextMenu.SubContent>
      </ContextMenu.Portal>
    </ContextMenu.Sub>
  {/if}
  {#if twoActive}
    <ContextMenu.Separator />
    <ContextMenu.Item onclick={() => tileSideBySideAndClose([sel[0], sel[1]])}>
      Tile Side by Side
    </ContextMenu.Item>
  {/if}
{:else if isMinimized}
  <ContextMenu.Item onclick={() => { restoreWindow(windowId); hideOverlay(); }}>
    Restore
  </ContextMenu.Item>
  <ContextMenu.Item onclick={() => closeMinimizedWindow(windowId)}>
    Close
  </ContextMenu.Item>
  {#if moveTargets.length > 0}
    <ContextMenu.Separator />
    <ContextMenu.Sub>
      <ContextMenu.SubTrigger>Move to</ContextMenu.SubTrigger>
      <ContextMenu.Portal>
        <ContextMenu.SubContent class="shell-popover">
          {#each moveTargets as ws, i (ws.id)}
            <ContextMenu.Item onclick={() => restoreWindowToWorkspace(windowId, ws.id)}>
              {ws.name || `Workspace ${i + 1}`}
            </ContextMenu.Item>
          {/each}
        </ContextMenu.SubContent>
      </ContextMenu.Portal>
    </ContextMenu.Sub>
  {/if}
{:else}
  <ContextMenu.Item onclick={() => closeWindowAction(windowId)}>Close</ContextMenu.Item>
  <ContextMenu.Item onclick={() => minimizeWindow(windowId)}>Minimize</ContextMenu.Item>
  {#if moveTargets.length > 0}
    <ContextMenu.Separator />
    <ContextMenu.Sub>
      <ContextMenu.SubTrigger>Move to</ContextMenu.SubTrigger>
      <ContextMenu.Portal>
        <ContextMenu.SubContent class="shell-popover">
          {#each moveTargets as ws, i (ws.id)}
            <ContextMenu.Item onclick={() => moveWindowToWorkspaceAction(windowId, ws.id)}>
              {ws.name || `Workspace ${i + 1}`}
            </ContextMenu.Item>
          {/each}
        </ContextMenu.SubContent>
      </ContextMenu.Portal>
    </ContextMenu.Sub>
  {/if}
  <ContextMenu.Separator />
  <ContextMenu.Item onclick={() => tileWindowAction(windowId, "left")}>
    Tile Left
  </ContextMenu.Item>
  <ContextMenu.Item onclick={() => tileWindowAction(windowId, "right")}>
    Tile Right
  </ContextMenu.Item>
  <ContextMenu.Item
    onclick={() => fullscreenWindowAction(windowId, win?.fullscreen ?? false)}
  >
    {win?.fullscreen ? "Exit Fullscreen" : "Fullscreen"}
  </ContextMenu.Item>
{/if}
