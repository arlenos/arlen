<script lang="ts">
  /// Quick Settings slider tile.
  ///
  /// 2×1 tile that embeds a range input below the label. Used for
  /// brightness, audio volume, mic level. The component coalesces
  /// drag updates into ~30Hz hardware writes via the caller's
  /// `oninput` callback so backends (logind, wpctl) don't get
  /// flooded; pass-through to the base BaseTile primitive otherwise.
  ///
  /// Optional detail strip — mirrors BaseTile's pattern. When
  /// `onDetail` is supplied the strip renders below the slider with
  /// the `statusText` as the row label and a chevron at the right.
  /// Used by Sound to surface the AudioPopover (output picker / per-
  /// app mixer). Brightness has no popover, so it omits the strip
  /// and renders the legacy two-row layout (head + slider).
  import type { Snippet } from "svelte";
  import { ChevronRight } from "@lucide/svelte";

  let {
    label,
    statusText = "",
    icon,
    value,
    min = 0,
    max = 100,
    step = 1,
    active = $bindable(true),
    disabled = false,
    oninput,
    onfocus,
    onblur,
    oncontextmenu,
    onDetail,
    detailLabel = "",
  }: {
    label: string;
    statusText?: string;
    icon?: Snippet;
    /// Current value in `[min, max]`. The component does not own
    /// state — the caller passes the live value and updates it from
    /// the backend.
    value: number;
    min?: number;
    max?: number;
    step?: number;
    /// Sliders are usually rendered "active" (accent fill) since
    /// they always have a value. Caller may toggle via `bind:active`.
    active?: boolean;
    disabled?: boolean;
    /// Fired on every range change. Caller decides whether to debounce
    /// hardware writes (typical pattern: setTimeout 32ms).
    oninput?: (value: number) => void;
    /// Fired when the slider input gains focus. The orchestrator uses
    /// this to set the focus-grid into "slider mode" so h/j/k/l flow
    /// to the range input instead of moving cell focus.
    onfocus?: () => void;
    /// Fired when the slider input loses focus. Counterpart to onfocus.
    onblur?: () => void;
    /// Right-click handler (suppresses the native context menu when
    /// supplied). Caller uses this to open detail surfaces — Sound
    /// tile right-click opens AudioPopover.
    oncontextmenu?: () => void;
    /// Detail-surface entry handler. When supplied, the status text
    /// moves out of the head row into a separate clickable strip
    /// below the slider, with its own hover bg + chevron. Click on
    /// the strip fires this handler. Mirrors BaseTile's `onDetail`
    /// affordance so the panel-wide pattern stays consistent.
    onDetail?: () => void;
    /// Override the strip's accessible label. Defaults to
    /// `"Open details for <label>"`.
    detailLabel?: string;
  } = $props();

  const percent = $derived(((value - min) / (max - min)) * 100);
  const stripAriaLabel = $derived(
    detailLabel || `Open details for ${label}`,
  );
</script>

<button
  type="button"
  class="qs-tile size-2x1"
  class:active
  class:has-strip={!!statusText}
  {disabled}
  tabindex="-1"
  onclick={(e) => e.preventDefault()}
  oncontextmenu={(e) => {
    if (oncontextmenu || onDetail) {
      e.preventDefault();
      (oncontextmenu ?? onDetail)?.();
    }
  }}
>
  <div class="qs-tile-head">
    {#if icon}
      <span class="qs-tile-icon">{@render icon()}</span>
    {/if}
    <div class="qs-tile-text">
      <span class="qs-tile-label">{label}</span>
    </div>
    <span class="qs-tile-value">{Math.round(percent)}%</span>
  </div>
  <div class="qs-slider" style="--value: {percent}%">
    <div class="qs-slider-track"></div>
    <div class="qs-slider-fill"></div>
    <div class="qs-slider-thumb"></div>
    <input
      type="range"
      {min}
      {max}
      {step}
      {value}
      {disabled}
      oninput={(e) => oninput?.(parseFloat(e.currentTarget.value))}
      onfocus={() => onfocus?.()}
      onblur={() => onblur?.()}
    />
  </div>

  {#if statusText}
    {#if onDetail}
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
      <!-- Passive strip: status info only, no chevron/hover/click.
           Same vertical position as the interactive variant so the
           status line is consistent across all SliderTile uses. -->
      <div class="qs-tile-strip">
        <span class="qs-tile-status">{statusText}</span>
      </div>
    {/if}
  {/if}
</button>

<style>
  .qs-tile {
    grid-column: span 2;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    min-height: 64px;
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: var(--radius-card);
    color: var(--color-fg-shell);
    cursor: default;
    text-align: left;
    overflow: hidden;
    transition: background-color 100ms ease, border-color 100ms ease;
  }
  .qs-tile:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
  }
  .qs-tile:focus-within {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 35%, transparent);
  }
  .qs-tile:disabled {
    opacity: 0.4;
  }

  /* Tighter padding when a strip is rendered — the strip owns its
     own bottom padding so the parent doesn't need to. */
  .qs-tile.has-strip {
    padding: 12px 12px 0 12px;
    gap: 8px;
  }

  .qs-tile-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .qs-tile-icon {
    display: inline-flex;
    color: var(--color-fg-shell);
    flex-shrink: 0;
  }
  .qs-tile-text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .qs-tile-label {
    font-size: 0.8125rem;
    font-weight: 500;
    line-height: 1.2;
  }
  .qs-tile-status {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .qs-tile-value {
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
  }

  .qs-slider {
    position: relative;
    height: 20px;
    display: flex;
    align-items: center;
  }
  .qs-slider-track {
    position: absolute;
    left: 0;
    right: 0;
    height: var(--slider-track-h, 4px);
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border-radius: var(--radius-chip);
  }
  .qs-slider-fill {
    position: absolute;
    left: 0;
    width: var(--value);
    height: 4px;
    background: var(--color-accent);
    border-radius: var(--radius-chip);
  }
  .qs-slider-thumb {
    position: absolute;
    left: var(--value);
    width: var(--slider-thumb-size, 14px);
    height: var(--slider-thumb-size, 14px);
    background: var(--color-fg-shell);
    border-radius: var(--radius-input);
    transform: translateX(-50%);
    box-shadow: var(--shadow-sm);
    pointer-events: none;
  }
  .qs-slider input[type="range"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
    margin: 0;
    appearance: none;
    -webkit-appearance: none;
  }

  /* Status strip — mirrors BaseTile pattern. Negative horizontal
     margin pulls the strip to the tile edges so the hover bg covers
     full width (matches BaseTile's overflow:hidden + edge-flush
     strip). Interactive vs passive variant only differs in cursor +
     chevron + hover treatment. */
  .qs-tile-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 12px;
    margin: 0 -12px;
    min-height: 32px;
    transition: background-color 100ms ease;
  }
  .qs-tile-strip.is-interactive {
    cursor: pointer;
  }
  .qs-tile-strip.is-interactive:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
  }
  .qs-tile-strip.is-interactive:focus-visible {
    outline: none;
    background: color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
  }
  .qs-tile.active .qs-tile-strip.is-interactive:hover {
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
  }

  :global(.qs-tile-chevron) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    transition: color 100ms ease, transform 100ms ease;
  }
  .qs-tile-strip.is-interactive:hover :global(.qs-tile-chevron) {
    color: var(--color-fg-shell);
    transform: translateX(2px);
  }
</style>
