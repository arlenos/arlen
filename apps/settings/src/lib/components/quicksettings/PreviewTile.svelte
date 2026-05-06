<script lang="ts">
  /// Schematic tile rendered inside the QS layout editor preview.
  ///
  /// Mimics the `BaseTile` / `SliderTile` shape but replaces every
  /// piece of live content (chart, slider track, status text) with
  /// `<Skeleton>` placeholders so the editor focuses on layout
  /// without flickering as background data arrives.
  ///
  /// Pinned tiles (currently only `system.user-row`) bypass the
  /// card frame entirely and render as a footer row that mirrors
  /// the real shell's `.user-row-footer` layout — avatar +
  /// name-skeleton + theme/settings icon-skeletons. This matches
  /// what the live panel actually shows: the user-row is NOT a
  /// card-tile in the shell, just a footer strip with no border.
  ///
  /// Drag uses pointer events instead of HTML5 drag-and-drop —
  /// the workspace indicator's debug-session notes (2026-04-19)
  /// confirm HTML5 drag freezes WebKitGTK when combined with
  /// custom ghosts. Pointer events give us full control. The
  /// orchestrator (PanelPreview) owns the drag-state machine and
  /// passes the relevant handlers as props.
  import { Skeleton } from "$lib/components/ui/skeleton";
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
    GripVertical,
    EyeOff,
    Maximize,
    Settings,
    Square as SquareIcon,
    Layout as LayoutIcon,
    User,
  } from "@lucide/svelte";

  type WireSize = "one_by_one" | "two_by_one" | "two_by_two";

  let {
    id,
    label,
    iconName,
    size,
    allowedSizes,
    bodyVariant,
    dragging = false,
    onPointerDown,
    onResize,
    onHide,
    readOnly = false,
    pinned = false,
  }: {
    id: string;
    label: string;
    iconName: string;
    size: WireSize;
    allowedSizes: WireSize[];
    bodyVariant: "status" | "slider" | "chart" | "footer";
    dragging?: boolean;
    /// Pointer-down handler; orchestrator decides whether to enter
    /// drag-mode or treat as a click. Skipped for pinned/readOnly
    /// tiles — they receive no drag glue.
    onPointerDown?: (e: PointerEvent, id: string) => void;
    onResize?: (id: string, size: WireSize) => void;
    onHide?: (id: string) => void;
    readOnly?: boolean;
    pinned?: boolean;
  } = $props();

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

  const Icon = $derived(ICONS[iconName] ?? SquareIcon);

  function sizeLabel(s: WireSize): string {
    return s === "two_by_two" ? "Large" : s === "two_by_one" ? "Wide" : "Standard";
  }
  function sizeIcon(s: WireSize): typeof Maximize {
    return s === "two_by_two" ? Maximize : s === "two_by_one" ? LayoutIcon : SquareIcon;
  }

  function handlePointerDown(e: PointerEvent) {
    if (readOnly || pinned) return;
    onPointerDown?.(e, id);
  }
</script>

{#if pinned}
  <!-- Pinned footer row: matches the real shell `.user-row-footer`
       (no card-frame, just a flex strip). The shell renders avatar
       + name + theme-toggle + settings-button on a single row; the
       editor mirrors that with skeleton placeholders for each
       slot. -->
  <div
    class="preview-footer-row"
    class:read-only={readOnly}
    role="group"
    aria-label={label}
    data-tile-id={id}
  >
    <Skeleton class="footer-avatar" />
    <Skeleton class="footer-name" />
    <span class="footer-actions">
      <Skeleton class="footer-action" />
      <Skeleton class="footer-action" />
    </span>
  </div>
{:else}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          {...props}
          class="preview-tile"
          class:size-1x1={size === "one_by_one"}
          class:size-2x1={size === "two_by_one"}
          class:size-2x2={size === "two_by_two"}
          class:dragging
          class:read-only={readOnly}
          role="group"
          aria-label={label}
          data-tile-id={id}
          onpointerdown={handlePointerDown}
        >
          <span class="grip-handle" aria-hidden="true">
            <GripVertical size={14} strokeWidth={1.5} />
          </span>
          <div class="preview-head">
            <span class="preview-icon">
              <Icon size={14} strokeWidth={1.75} />
            </span>
            <span class="preview-label">{label}</span>
          </div>
          {#if bodyVariant === "slider"}
            <div class="preview-body">
              <Skeleton class="skeleton-slider" />
            </div>
          {:else if bodyVariant === "chart"}
            <div class="preview-body">
              <Skeleton class="skeleton-chart" />
            </div>
          {:else}
            <div class="preview-body">
              <Skeleton class="skeleton-status" />
            </div>
          {/if}
        </div>
      {/snippet}
    </ContextMenu.Trigger>
    {#if !readOnly}
      <ContextMenu.Content>
        {#if allowedSizes.length > 1}
          <ContextMenu.Sub>
            <ContextMenu.SubTrigger>
              {@const SizeIcon = sizeIcon(size)}
              <SizeIcon size={14} strokeWidth={1.5} />
              <span>Size: {sizeLabel(size)}</span>
            </ContextMenu.SubTrigger>
            <ContextMenu.SubContent>
              {#each allowedSizes as opt (opt)}
                {@const OptIcon = sizeIcon(opt)}
                <ContextMenu.Item
                  onclick={() => onResize?.(id, opt)}
                  disabled={opt === size}
                >
                  <OptIcon size={14} strokeWidth={1.5} />
                  <span>{sizeLabel(opt)}</span>
                </ContextMenu.Item>
              {/each}
            </ContextMenu.SubContent>
          </ContextMenu.Sub>
          <ContextMenu.Separator />
        {/if}
        <ContextMenu.Item onclick={() => onHide?.(id)}>
          <EyeOff size={14} strokeWidth={1.5} />
          <span>Hide tile</span>
        </ContextMenu.Item>
      </ContextMenu.Content>
    {/if}
  </ContextMenu.Root>
{/if}

<style>
  .preview-tile {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    min-height: 64px;
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: var(--radius-card);
    color: var(--color-fg-shell);
    cursor: grab;
    user-select: none;
    touch-action: none;
    transition:
      background-color 100ms ease,
      border-color 100ms ease,
      opacity 100ms ease;
  }
  .preview-tile:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
  }
  .preview-tile:active {
    cursor: grabbing;
  }
  .preview-tile.dragging {
    opacity: 0.4;
  }
  .preview-tile.read-only {
    cursor: not-allowed;
    opacity: 0.6;
  }

  .preview-tile.size-1x1 {
    grid-column: span 1;
    grid-row: span 1;
  }
  .preview-tile.size-2x1 {
    grid-column: span 2;
    grid-row: span 1;
  }
  .preview-tile.size-2x2 {
    grid-column: span 2;
    grid-row: span 2;
  }

  .grip-handle {
    position: absolute;
    top: 6px;
    right: 6px;
    color: color-mix(in srgb, var(--color-fg-shell) 35%, transparent);
    pointer-events: none;
    transition: color 100ms ease;
  }
  .preview-tile:hover .grip-handle {
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
  }

  .preview-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 22px;
  }
  .preview-icon {
    display: inline-flex;
    flex-shrink: 0;
  }
  .preview-label {
    font-size: 0.8125rem;
    font-weight: 500;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .preview-body {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  :global(.skeleton-status) {
    height: 10px;
    width: 60%;
  }
  :global(.skeleton-slider) {
    height: 4px;
    width: 100%;
  }
  :global(.skeleton-chart) {
    height: 24px;
    width: 100%;
  }

  /* Footer-row variant for pinned tiles. Matches the real shell
     `.user-row-footer` styling: no card border/bg, just a flex
     strip with a top separator. */
  .preview-footer-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 4px 0 4px;
    margin-top: 4px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
  .preview-footer-row.read-only {
    opacity: 0.6;
  }
  :global(.footer-avatar) {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-card);
    flex-shrink: 0;
  }
  :global(.footer-name) {
    height: 12px;
    width: 90px;
    flex-shrink: 0;
  }
  .footer-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: auto;
  }
  :global(.footer-action) {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-input);
  }
</style>
