<script lang="ts">
  /// One window card in the workspace overlay — active or minimized
  /// variant. The card is the drag source (pointer handlers forward
  /// into the drag engine) and the right-click anchor for the
  /// window context menu.

  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu/index.js";
  import { AppWindow } from "lucide-svelte";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import type { DragEngine } from "$lib/workspace/drag.svelte.js";
  import { truncateTitle } from "$lib/workspace/format.js";
  import WindowCardMenu from "./WindowCardMenu.svelte";

  let {
    windowId,
    wsId,
    wsIndex,
    title,
    appId,
    minimized = false,
    iconUrl = null,
    selected,
    keyboardFocus,
    dragging,
    drag,
    workspaces,
    hideOverlay,
    onMenuOpenChange,
  }: {
    windowId: string;
    wsId: string;
    /// 0-based column index; aria labels speak 1-based.
    wsIndex: number;
    title: string;
    appId: string;
    minimized?: boolean;
    iconUrl?: string | null;
    selected: boolean;
    keyboardFocus: boolean;
    dragging: boolean;
    drag: DragEngine;
    /// Passed through to the menu for its "Move to" targets.
    workspaces: WorkspaceInfo[];
    /// State-only overlay close, passed through to the menu.
    hideOverlay: () => void;
    /// Menu open/close tracker for the hover engine. Only the
    /// active cards wire it today — the minimized cards never did;
    /// the asymmetry is conserved as-is from the monolith.
    onMenuOpenChange?: (open: boolean) => void;
  } = $props();
</script>

<ContextMenu.Root onOpenChange={onMenuOpenChange}>
  <ContextMenu.Trigger>
    {#snippet child({ props })}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <button
        {...props}
        class="window-card"
        class:window-card-minimized={minimized}
        class:window-card-dragging={dragging}
        class:window-card-keyboard-focus={keyboardFocus}
        class:window-card-selected={selected}
        onpointerdown={(e) =>
          drag.onCardPointerDown(
            e,
            windowId,
            wsId,
            minimized ? "minimized" : "active",
          )}
        onpointermove={drag.onCardPointerMove}
        onpointerup={drag.onCardPointerUp}
        onpointercancel={drag.onCardPointerCancel}
        title={title || appId}
        aria-label={minimized
          ? `Minimized: ${title || appId} on workspace ${wsIndex + 1}`
          : `${title || appId} on workspace ${wsIndex + 1}`}
      >
        {#if iconUrl}
          <img
            class="window-card-icon"
            src={iconUrl}
            alt=""
            width="24"
            height="24"
            draggable="false"
          />
        {:else}
          <AppWindow
            size={20}
            strokeWidth={1.5}
            class="window-card-icon-fallback"
          />
        {/if}
        <span class="window-card-title">
          {truncateTitle(title, appId)}
        </span>
      </button>
    {/snippet}
  </ContextMenu.Trigger>
  <ContextMenu.Portal>
    <ContextMenu.Content class="shell-popover">
      <WindowCardMenu
        {windowId}
        isMinimized={minimized}
        {workspaces}
        {hideOverlay}
      />
    </ContextMenu.Content>
  </ContextMenu.Portal>
</ContextMenu.Root>

<style>
  /* Card format register: active 60×56, minimized 48×44 (overrides
     at the bottom of this file — their source order is load-bearing,
     see the comment there). */
  .window-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    width: 60px;
    height: 56px;
    padding: 8px 4px;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    cursor: grab;
    color: var(--color-fg-shell);
    transition:
      transform var(--duration-micro, 100ms) ease,
      background-color var(--duration-micro, 100ms) ease,
      opacity var(--duration-micro, 100ms) ease;
  }

  .window-card:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    transform: scale(1.03);
  }

  .window-card:active {
    cursor: grabbing;
  }

  .window-card-dragging {
    /* Source card stays as a faint placeholder — the ghost clone
       carries the pointer-following visual. Scale override comes
       from the `:hover` rule below which we also suppress. */
    opacity: 0.3;
  }
  .window-card-dragging:hover {
    transform: none;
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
  }

  /* Keyboard-navigation focus ring. Distinct from `.ws-column-active`
     (subtle accent tint on the whole column) — this is a saturated
     accent outline directly on the focused card so it stands out
     even inside the active column. */
  .window-card-keyboard-focus {
    border-color: var(--color-accent);
    box-shadow:
      0 0 0 2px color-mix(in srgb, var(--color-accent) 50%, transparent);
  }
  .window-card-keyboard-focus:hover {
    border-color: var(--color-accent);
  }

  /* Multi-selection ring. Accent border + accent-tinted background
     so the selection reads as distinct from hover (neutral tint)
     and keyboard focus (thin solid ring). A selected card that is
     also keyboard-focused uses the focus ring on top — the
     selection background still shows through. */
  .window-card-selected {
    border-color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
  }
  .window-card-selected:hover {
    background: color-mix(in srgb, var(--color-accent) 24%, transparent);
  }

  .window-card-icon {
    width: 24px;
    height: 24px;
    object-fit: contain;
    border-radius: var(--radius-chip);
    pointer-events: none;
  }

  :global(.window-card-icon-fallback) {
    opacity: 0.5;
  }

  /* ── Drag ghost ──────────────────────────────────────────────────────
     The ghost is appended to `document.body`, outside the component's
     scoped DOM subtree, and JS owns its whole lifecycle — the float
     look is applied as inline styles in `applyGhostFloatStyle` (the
     only layer that beats the scoped `.window-card` rules the clone
     carries). Only the overflow badge, a fresh span with nothing to
     fight, is styled here. */
  :global(.drag-ghost-badge) {
    position: absolute;
    right: -6px;
    bottom: -6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--color-accent);
    /* The accent is a light monochrome in the default theme — plain
       `white` washes out on it; the inverse foreground keeps the
       count readable on any accent. */
    color: var(--color-fg-inverse);
    font-size: 11px;
    font-weight: 600;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.35);
    z-index: 1;
  }

  .window-card-title {
    font-size: 10px;
    line-height: 1.1;
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  /* ── Minimized card overrides ──────────────────────────────────────
     These MUST come after the .window-card / .window-card-icon /
     .window-card-title blocks above. Svelte scopes both `.window-card`
     and `.window-card-minimized` to the same component hash, giving
     them equal specificity (0,2,0). Source order then decides the
     tie — so the more-specific-looking dual-class selector only wins
     if it's declared later in the file.
     `:global(...)` is used so the ghost clone in document.body (which
     doesn't carry the component's scope hash) still gets the size
     override during drag. The dual-class selector inside `:global`
     has specificity (0,2,0), same as the scoped `.window-card`, so
     the source-order rule applies there too. */
  :global(.window-card.window-card-minimized) {
    width: 48px;
    height: 44px;
    padding: 6px 3px;
    gap: 3px;
    opacity: 0.72;
    transition:
      transform var(--duration-micro, 100ms) ease,
      background-color var(--duration-micro, 100ms) ease,
      opacity var(--duration-fast, 150ms) ease;
  }
  :global(.window-card.window-card-minimized:hover) {
    opacity: 1;
  }
  :global(.window-card.window-card-minimized .window-card-icon) {
    width: 18px;
    height: 18px;
  }
  :global(.window-card.window-card-minimized .window-card-title) {
    font-size: 9px;
    line-height: 1.05;
  }
</style>
