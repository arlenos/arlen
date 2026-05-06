<script lang="ts">
  /// Drawer for tiles the user has hidden from the QS panel.
  ///
  /// Collapsible section under the panel preview. Each hidden
  /// tile shows as a chip — drag back into the preview to un-
  /// hide, or click "Show" in the chip's right-click menu to
  /// append at the end.
  import { ChevronDown, Eye } from "@lucide/svelte";
  import * as ContextMenu from "$lib/components/ui/context-menu";
  import {
    Brain,
    Folder,
    Wifi,
    Bluetooth,
    BellOff,
    Plane,
    Sun,
    Volume2,
    User,
    Square,
  } from "@lucide/svelte";

  interface HiddenTile {
    id: string;
    label: string;
    iconName: string;
  }

  let {
    tiles,
    onShow,
  }: {
    tiles: HiddenTile[];
    onShow: (id: string) => void;
  } = $props();

  let open = $state(true);

  /// Same icon table as PreviewTile — kept inline for now;
  /// future Phase-7 module-tier will inject icons through a
  /// shared registry both files read from.
  const ICONS: Record<string, typeof Brain> = {
    brain: Brain,
    folder: Folder,
    wifi: Wifi,
    bluetooth: Bluetooth,
    "bell-off": BellOff,
    plane: Plane,
    sun: Sun,
    "volume-2": Volume2,
    user: User,
  };

  function handleDragStart(e: DragEvent, id: string) {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", id);
  }
</script>

{#if tiles.length > 0}
  <div class="hidden-drawer">
    <button class="drawer-header" onclick={() => (open = !open)}>
      <ChevronDown
        size={14}
        strokeWidth={1.75}
        class={open ? "drawer-chev open" : "drawer-chev"}
      />
      <span>Hidden tiles</span>
      <span class="drawer-count">{tiles.length}</span>
    </button>
    {#if open}
      <div class="drawer-body">
        {#each tiles as tile (tile.id)}
          {@const Icon = ICONS[tile.iconName] ?? Square}
          <ContextMenu.Root>
            <ContextMenu.Trigger>
              <div
                class="hidden-chip"
                role="button"
                tabindex="0"
                aria-label={`Hidden tile: ${tile.label} — drag back into the panel or right-click to show`}
                draggable="true"
                ondragstart={(e) => handleDragStart(e, tile.id)}
              >
                <Icon size={14} strokeWidth={1.75} />
                <span>{tile.label}</span>
              </div>
            </ContextMenu.Trigger>
            <ContextMenu.Content>
              <ContextMenu.Item onclick={() => onShow(tile.id)}>
                <Eye size={14} strokeWidth={1.5} />
                <span>Show in panel</span>
              </ContextMenu.Item>
            </ContextMenu.Content>
          </ContextMenu.Root>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .hidden-drawer {
    margin-top: 16px;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-shell) 3%, transparent);
    overflow: hidden;
  }
  .drawer-header {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 10px 12px;
    background: transparent;
    border: none;
    color: var(--color-fg-shell);
    font-size: 0.8125rem;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: background-color 100ms ease;
  }
  .drawer-header:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
  }
  :global(.drawer-chev) {
    transition: transform 150ms ease;
    transform: rotate(-90deg);
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
  }
  :global(.drawer-chev.open) {
    transform: rotate(0deg);
  }
  .drawer-count {
    margin-left: auto;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    padding: 0 8px;
    border-radius: var(--radius-full, 9999px);
    line-height: 1.7;
    font-variant-numeric: tabular-nums;
  }

  .drawer-body {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 4px 12px 12px 12px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
  }
  .hidden-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
    border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--color-fg-shell) 75%, transparent);
    font-size: 0.75rem;
    cursor: grab;
    transition: background-color 100ms ease, color 100ms ease;
  }
  .hidden-chip:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
    color: var(--color-fg-shell);
  }
  .hidden-chip:active {
    cursor: grabbing;
  }
</style>
