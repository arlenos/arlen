<script lang="ts">
  /// Quick Settings tile primitive.
  ///
  /// Renders one cell in the QS panel grid. Holds two pieces of state
  /// the tile catalogue and the status channel control: an `active`
  /// boolean (drives accent colouring) and a `statusText` string (the
  /// subtitle under the label).
  ///
  /// Two click affordances:
  ///   * Body click  → primary toggle (caller's `onclick`).
  ///   * Detail strip click → secondary "open detail surface"
  ///     (caller's `onDetail`). Only rendered when `onDetail` is
  ///     supplied; the strip sits below the head with its own hover
  ///     bg, separator, and a chevron at the right edge.
  ///
  /// `oncontextmenu` is kept as an alternative entry to the same
  /// detail surface (right-click power-shortcut + keyboard
  /// ContextMenu key).
  ///
  /// Sizes:
  ///   `1x1` — half-row (default). Square-ish tile.
  ///   `2x1` — full-row, single height. Used for sliders and
  ///           context-rich tiles (project context).
  ///   `2x2` — full-row, double height. Reserved for tiles that
  ///           expose inline detail content (audio media controls).
  import type { Snippet } from "svelte";
  import { ChevronRight } from "@lucide/svelte";

  type Size = "1x1" | "2x1" | "2x2";

  let {
    label,
    statusText = "",
    active = false,
    icon,
    size = "1x1",
    disabled = false,
    onclick,
    oncontextmenu,
    onDetail,
    detailLabel = "",
    headTrailing,
    children,
    tabindex,
  }: {
    /// Primary tile label, shown above the status subtitle.
    label: string;
    /// Subtitle line. Empty string hides the line entirely.
    statusText?: string;
    /// `true` paints the tile in accent colour; `false` is the
    /// neutral resting state. Tiles with `click = "noop"` may still
    /// pass `active = true` to indicate state without being
    /// clickable.
    active?: boolean;
    /// Icon snippet (the caller passes `<Wifi size={16} />` etc.).
    /// Renders in the top-left.
    icon?: Snippet;
    /// Cell size in the logical grid.
    size?: Size;
    /// `true` greys out the tile and prevents clicks. Used by
    /// `available_when` failures (e.g. brightness tile with no
    /// backlight device).
    disabled?: boolean;
    /// Click handler for the primary body. Caller decides what
    /// "primary" means — for Network it's WiFi-toggle, for DND it's
    /// flip the suppress mode, etc.
    onclick?: () => void;
    /// Right-click handler. Called on `contextmenu` event. The
    /// native context menu is suppressed only when a handler is
    /// supplied.
    oncontextmenu?: () => void;
    /// Detail-surface entry handler. When supplied, the status row
    /// becomes a separate clickable strip with its own hover bg,
    /// separator, and chevron. This is the discoverable counterpart
    /// to `oncontextmenu` — same target, mouse-visible affordance.
    onDetail?: () => void;
    /// Override the strip's accessible label. Defaults to
    /// `"Open details for <label>"`. Useful when the tile's name
    /// alone doesn't tell the user what surface opens.
    detailLabel?: string;
    /// Optional trailing content for the head row, right-aligned
    /// after the label. SliderTile renders its percent readout here.
    headTrailing?: Snippet;
    /// Optional inline content rendered between the head and the
    /// strip. SliderTile embeds its slider here.
    children?: Snippet;
    /// Focus override for the outer button. SliderTile passes -1:
    /// its inner slider is the keyboard stop, not the tile body.
    tabindex?: number;
  } = $props();

  const stripAriaLabel = $derived(
    detailLabel || `Open details for ${label}`,
  );
</script>

<!-- The outer is the focusable primary toggle. The inner detail
     strip is a div with role="button" + tabindex=-1 so mouse users
     get the affordance without making focus-grid traversal land on
     two stops per tile. Keyboard users reach the detail surface via
     the ContextMenu key (Shift+F10), which fires the same
     `oncontextmenu` handler. -->
<button
  type="button"
  class="qs-tile"
  class:active
  class:size-2x1={size === "2x1"}
  class:size-2x2={size === "2x2"}
  class:has-strip={!!statusText}
  {disabled}
  {tabindex}
  onclick={() => onclick?.()}
  oncontextmenu={(e) => {
    if (oncontextmenu || onDetail) {
      e.preventDefault();
      (oncontextmenu ?? onDetail)?.();
    }
  }}
  aria-pressed={active}
>
  <div class="qs-tile-head">
    {#if icon}
      <span class="qs-tile-icon">{@render icon()}</span>
    {/if}
    <div class="qs-tile-text">
      <span class="qs-tile-label">{label}</span>
    </div>
    {#if headTrailing}
      <span class="qs-tile-head-trailing">{@render headTrailing()}</span>
    {/if}
  </div>

  {#if children}
    <div class="qs-tile-body">
      {@render children()}
    </div>
  {/if}

  {#if statusText}
    {#if onDetail}
      <!-- Interactive strip: chevron + hover-bg + own click target.
           role=button + aria-label give screen readers an
           announcement; tabindex=-1 keeps focus-grid traversal
           single-stop per tile. Mouse click stops propagation so the
           outer toggle doesn't also fire. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="qs-tile-strip is-interactive"
        role="button"
        tabindex="-1"
        aria-label={stripAriaLabel}
        onclick={(e) => {
          e.stopPropagation();
          onDetail();
        }}
      >
        <span class="qs-tile-status">{statusText}</span>
        <ChevronRight size={14} strokeWidth={1.75} class="qs-tile-chevron" />
      </div>
    {:else}
      <!-- Passive strip: same vertical position as the interactive
           variant so the status line lives in a consistent place
           across all tiles. No chevron, no hover, no cursor change —
           the lack of those signals "info, not action". -->
      <div class="qs-tile-strip">
        <span class="qs-tile-status">{statusText}</span>
      </div>
    {/if}
  {/if}

</button>

<style>
  .qs-tile {
    /* Base 1x1 cell. */
    grid-column: span 1;
    grid-row: span 1;
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    gap: 0;
    padding: 0;
    min-height: 64px;
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    border-radius: var(--radius-card);
    color: var(--foreground);
    text-align: left;
    overflow: hidden;
    transition: background-color var(--duration-micro, 100ms) ease, border-color var(--duration-micro, 100ms) ease;
  }
  .qs-tile.size-2x1 {
    grid-column: span 2;
  }
  .qs-tile.size-2x2 {
    grid-column: span 2;
    grid-row: span 2;
  }

  .qs-tile:hover:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 20%, transparent);
  }

  .qs-tile:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 35%, transparent);
  }

  /* Active state: the system-wide "connected/active item" pattern
     (hover 10%, active 15% bg + 30% border) shared with the
     Bluetooth/Network popover rows. */
  .qs-tile.active {
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
  }
  .qs-tile.active:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-accent) 22%, transparent);
  }

  .qs-tile:disabled {
    opacity: 0.4;
  }

  .qs-tile-head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    min-height: var(--height-control-prominent, 36px);
  }
  .qs-tile.has-strip .qs-tile-head {
    /* Tighter when followed by a strip — the head no longer carries
       the status line, only the icon + label. */
    padding-bottom: 10px;
  }

  .qs-tile-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--foreground);
    flex-shrink: 0;
  }
  .qs-tile.active .qs-tile-icon {
    color: var(--color-accent);
  }

  .qs-tile-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .qs-tile-head-trailing {
    flex-shrink: 0;
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .qs-tile-label {
    font-size: 0.8125rem;
    font-weight: 500;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .qs-tile-status {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Status strip — always rendered when statusText is set, so the
     status line lives in the same vertical position across all
     tiles. The interactive variant adds chevron + hover-bg + click
     target; the passive variant is just the line. No top-border in
     either case: zone-differentiation reads via the hover bg-step
     (interactive) or simply via being the bottom-of-tile region
     (passive), and avoids a fragile cross-tint line that fights
     active-state saturation. */
  .qs-tile-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 12px;
    /* Strip uses --height-control (28) rather than the head's
       --height-control-prominent (36): hierarchy comes from this
       step-down, plus the smaller status-text font, plus the
       hover-bg differentiation. Token-bound so a future scale
       revision moves both head and strip together. */
    min-height: var(--height-control, 28px);
    margin-top: auto;
    transition: background-color var(--duration-micro, 100ms) ease;
  }
  .qs-tile-strip.is-interactive:hover {
    background: color-mix(in srgb, var(--foreground) 14%, transparent);
  }
  .qs-tile.active .qs-tile-strip.is-interactive:hover {
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
  }

  :global(.qs-tile-chevron) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    transition: color var(--duration-micro, 100ms) ease, transform var(--duration-micro, 100ms) ease;
  }
  .qs-tile-strip.is-interactive:hover :global(.qs-tile-chevron) {
    color: var(--foreground);
    transform: translateX(2px);
  }

  .qs-tile-body {
    display: flex;
    flex-direction: column;
    padding: 0 12px;
  }
</style>
