<script lang="ts">
  /// Quick Settings panel orchestrator.
  ///
  /// Reads the user's `~/.config/arlen/quicksettings.toml` layout via
  /// `qs_layout_get`, merges with the bundled-defaults catalogue, and
  /// renders the resulting tile list in a 2-column logical grid. Each
  /// tile's behaviour (toggle/popover/flyout/slider) lives inside the
  /// individual tile components in `lib/quicksettings/tiles/`.
  ///
  /// The panel is a popover in the global `activePopover` store. It
  /// closes on backdrop click and on `Escape` (handled by the
  /// `focusGrid` keyboard helper from `@arlen/ui-kit/keyboard`).
  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { toast } from "svelte-sonner";
  import { focusGrid } from "@arlen/ui-kit/keyboard";
  import { resolveLayout, sizeFromWire, type LayoutEntry, type ResolvedTile, type WireSize } from "$lib/quicksettings/grid.js";

  interface RawTileEntry {
    id: string;
    visible: boolean;
    size: WireSize;
  }
  interface RawLayoutFile {
    tile?: RawTileEntry[];
    tiles?: RawTileEntry[];
  }

  let userEntries = $state<LayoutEntry[]>([]);
  let resolvedTiles = $derived(resolveLayout(userEntries));
  let helpOpen = $state(false);
  let panelEl: HTMLElement | undefined = $state();

  /// Coalesces watcher-driven reloads. The notify watcher debounces
  /// at 120ms and atomic-rename writes can still emit multiple
  /// events per save; the frontend stacks another 250ms here so a
  /// rapid-fire sequence of writes (e.g. user hammering the
  /// drag-drop in app-settings) lands as a single re-render rather
  /// than nine of them. This is the user-visible "shell freezes
  /// for ~5sec while panel updates" — many cascading re-renders
  /// across nine tile components compound far worse than they
  /// should.
  const RELOAD_DEBOUNCE_MS = 250;
  let reloadTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleReload() {
    if (reloadTimer) clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => {
      reloadTimer = null;
      loadLayout();
    }, RELOAD_DEBOUNCE_MS);
  }

  onMount(() => {
    loadLayout();
    // Watcher events: shell.toml changes (night-light, brightness)
    // and quicksettings.toml changes (layout edits). Both call the
    // debounced scheduleReload so back-to-back saves don't cascade
    // into back-to-back full panel re-renders.
    let stops: UnlistenFn[] = [];
    listen("arlen://shell-config-changed", scheduleReload).then((u) =>
      stops.push(u),
    );
    listen("arlen://qs-layout-changed", scheduleReload).then((u) =>
      stops.push(u),
    );
    return () => {
      for (const s of stops) s();
      if (reloadTimer) clearTimeout(reloadTimer);
    };
  });

  /// Tracks whether we've successfully loaded the user file at
  /// least once. On the very first load, an error means we have no
  /// known-good state — fall through to bundled defaults so the
  /// user sees a working panel. On subsequent reloads (triggered
  /// by the shell-config-changed file watch), an error means the
  /// user's customisation just got corrupted somehow; we keep the
  /// last-known-good state in `userEntries` rather than silently
  /// reverting to defaults (Codex review HIGH-3 / medium-3).
  /// Either way we surface a visible warning toast — without it
  /// the failure looks like a "reset to defaults" mystery.
  let hasLoadedOnce = $state(false);

  async function loadLayout() {
    try {
      const raw = await invoke<RawLayoutFile>("qs_layout_get");
      const arr = raw.tile ?? raw.tiles ?? [];
      userEntries = arr.map((e) => ({
        id: e.id,
        visible: e.visible,
        size: sizeFromWire(e.size),
      }));
      hasLoadedOnce = true;
    } catch (err) {
      console.warn("qs_layout_get failed:", err);
      const isParseError = String(err).includes("parse:");
      if (!hasLoadedOnce) {
        userEntries = [];
      }
      // else: keep the last-known-good `userEntries` rendered.
      toast.warning(
        isParseError
          ? "Quick Settings layout file is malformed. Using "
            + (hasLoadedOnce ? "last good state" : "defaults")
            + ". Fix or reset the file in Settings."
          : "Could not read Quick Settings layout: " + err,
        { duration: 8000 },
      );
    }
  }

  /// Build the focus-grid cells snapshot every key-press: tile DOM may
  /// re-render when state changes.
  function gridCells() {
    if (!panelEl) return [];
    const elements = panelEl.querySelectorAll<HTMLElement>(
      ".qs-tile, .user-row-trigger",
    );
    return Array.from(elements).map((el) => {
      const fullRow = el.classList.contains("size-2x1") ||
                      el.classList.contains("size-2x2") ||
                      el.closest(".audio-tile-wrap, .user-row-tile") !== null;
      return { el, spanCols: (fullRow ? 2 : 1) as 1 | 2 };
    });
  }
</script>

<!-- Backdrop: only mounted while open. Lightweight DOM, no state
     to preserve. -->
{#if $activePopover === "quick-settings"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="qs-backdrop" onclick={() => closePopover()}></div>
{/if}

<!-- Panel: ALWAYS mounted, visibility toggled via class. Each tile
     polls / subscribes to its data source on first mount; the
     `{#if}` model from before unmounted the panel on close, which
     forced every tile to re-poll on open (and briefly flash
     "off" because state was null until the network/bluetooth/
     audio call returned). Persistent mounting keeps event
     listeners attached across opens and lands the panel
     pre-populated. -->
{#if true}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    bind:this={panelEl}
    class="qs-panel shell-popover"
    class:visible={$activePopover === "quick-settings"}
    use:focusGrid={{
      cells: gridCells,
      onEscape: closePopover,
      onHelp: () => (helpOpen = !helpOpen),
    }}
  >
    {#if resolvedTiles.length === 0}
      <div class="qs-empty">
        No tiles enabled. Open
        <button
          class="qs-link"
          onclick={() =>
            invoke("quick_action_run", { id: "qa.open_settings_appearance" }).catch(() => {})}
        >
          Settings → Quick Settings
        </button>
        to enable some.
      </div>
    {:else}
      <div class="qs-grid">
        {#each resolvedTiles as t (t.id)}
          {@const Tile = t.component}
          <div class="qs-grid-cell" class:full-row={t.fullRow}>
            <Tile size={t.size} />
          </div>
        {/each}
      </div>
    {/if}

    {#if helpOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="qs-help-backdrop"
        onclick={(e) => {
          e.stopPropagation();
          helpOpen = false;
        }}
      ></div>
      <div class="qs-help">
        <h3>Keyboard</h3>
        <ul>
          <li><kbd>↑↓←→</kbd> / <kbd>hjkl</kbd> &nbsp;Navigate tiles</li>
          <li><kbd>Enter</kbd> / <kbd>Space</kbd> &nbsp;Activate</li>
          <li><kbd>Tab</kbd> &nbsp;Next focusable</li>
          <li><kbd>?</kbd> &nbsp;Toggle this help</li>
          <li><kbd>Esc</kbd> &nbsp;Close panel</li>
        </ul>
      </div>
    {/if}
  </div>
{/if}

<style>
  .qs-backdrop { position: fixed; inset: 0; z-index: 90; }

  /* Panel is permanently mounted. The `.visible` class drives the
     reveal — opacity + transform + pointer-events transition in,
     `visibility: hidden` (delayed) finishes the close so the panel
     is fully off-flow when not open. Tiles inside stay alive
     across open/close cycles which means no flash of "off" state
     while live data re-fetches. */
  .qs-panel {
    position: fixed;
    top: 40px;
    right: 8px;
    z-index: 100;
    width: 380px;
    max-height: calc(100vh - 56px);
    overflow-y: auto;
    border-radius: var(--radius-card);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    transform-origin: top center;
    /* Hidden default state. */
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    transform: translateY(-4px) scale(0.98);
    transition:
      opacity var(--duration-medium) var(--ease-out),
      transform var(--duration-medium) var(--ease-out),
      visibility 0s linear var(--duration-medium);
  }
  .qs-panel.visible {
    opacity: 1;
    visibility: visible;
    pointer-events: auto;
    transform: translateY(0) scale(1);
    /* On opening, visibility flips instantly so the transitions
       on opacity + transform actually run (otherwise the element
       is hidden the whole way). */
    transition:
      opacity var(--duration-medium) var(--ease-out),
      transform var(--duration-medium) var(--ease-out),
      visibility 0s linear 0s;
  }

  .qs-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 8px;
  }

  .qs-grid-cell {
    display: flex;
    min-width: 0;
  }
  .qs-grid-cell.full-row {
    grid-column: span 2;
  }
  /* The tile components own their inner padding; the grid-cell wrapper
     only positions them. The :global is needed because tiles render in
     ui-kit's scope, not desktop-shell's. */
  .qs-grid-cell :global(.qs-tile) {
    width: 100%;
  }
  .qs-grid-cell.full-row :global(.qs-tile) {
    grid-column: span 2;
  }

  .qs-empty {
    padding: 24px 12px;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    font-size: var(--text-sm);
    line-height: 1.5;
    text-align: center;
  }
  .qs-link {
    display: inline;
    background: transparent;
    border: none;
    color: var(--color-accent);
    padding: 0;
    font: inherit;
    text-decoration: underline;
  }

  .qs-help-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, black 30%, transparent);
    z-index: 110;
  }
  .qs-help {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 120;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border-radius: var(--radius-modal);
    padding: 16px 20px;
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    min-width: 260px;
  }
  .qs-help h3 {
    margin: 0 0 8px 0;
    font-size: var(--text-base);
    font-weight: 600;
  }
  .qs-help ul {
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
  }
  .qs-help li {
    padding: 4px 0;
    color: color-mix(in srgb, var(--color-fg-shell) 80%, transparent);
  }
  .qs-help kbd {
    display: inline-block;
    padding: 1px 6px;
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: var(--radius-chip);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    margin-right: 4px;
  }
</style>
