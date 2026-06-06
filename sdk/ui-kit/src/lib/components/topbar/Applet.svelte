<script lang="ts">
  /// Topbar Applet primitive.
  ///
  /// The canonical "thing in the topbar" that every indicator,
  /// trigger, and module-side applet routes through. Shell owns the
  /// chrome (button frame, hover, focus, tooltip, badge layering,
  /// hit-target sizing) so plugin authors physically can't ship a
  /// visually divergent applet — they only provide content (icon,
  /// optional inline label, optional badge, click handler).
  ///
  /// Sizing comes from the shell-level CSS tokens declared on the
  /// topbar root (`--topbar-applet-h`, `--topbar-applet-min-w`,
  /// `--topbar-applet-radius`, etc.), so themes / future "compact /
  /// comfortable" topbar variants update every applet at once.
  ///
  /// State naming is explicit (no overloaded `active` prop):
  ///   * `popoverOpen` — the applet's popover surface is currently
  ///     visible. Drives the accent-tinted "selected" look.
  ///   * `state` — semantic underlying status:
  ///       "on"        — connected / enabled / playing
  ///       "off"       — disconnected / disabled
  ///       "connecting"— transient pending state
  ///       "warn"      — caution (e.g. low battery)
  ///       "error"     — critical (e.g. unreachable adapter)
  ///   * `dimmed` — render at 40% opacity to signal low-priority /
  ///     not-currently-relevant. Independent of `state`.
  ///   * `disabled` — non-interactive (no click). Tooltip still
  ///     shows so the user can read why.
  ///
  /// Tooltip behaviour:
  ///   * Suppressed entirely while `popoverOpen` is true (the
  ///     popover is the explanation).
  ///   * Default 1500ms delay (matches CLAUDE.md tooltip convention),
  ///     pass `tooltipInstant` for 0ms.
  ///
  /// Badge slot is z-index-lifted above the icon's opacity layer so
  /// a `dimmed` applet still shows a crisp badge.
  import type { Snippet } from "svelte";
  import * as Tooltip from "../ui/tooltip";

  export type AppletState = "on" | "off" | "connecting" | "warn" | "error";

  let {
    icon,
    label,
    labelSnippet,
    badge,
    tooltip,
    tooltipInstant = false,
    popoverOpen = false,
    state,
    dimmed = false,
    disabled = false,
    appletId,
    onclick,
    onmouseenter,
    oncontextmenu,
    onWheel,
    ariaLabel,
  }: {
    /// Icon snippet rendered in the leading slot. Sized via
    /// `--topbar-applet-icon-size` (14px default).
    icon?: Snippet;
    /// Optional inline label rendered to the right of the icon.
    /// Used by Clock (text-only) and future MPRIS-expanded Audio.
    /// Truncates with ellipsis at `--topbar-applet-label-max-w`
    /// (120px default).
    label?: string;
    /// Escape-hatch for callers that need richer label content than
    /// a single styled string — e.g. Clock with a dim weekday +
    /// bright time on the same row. When provided, takes precedence
    /// over the `label` string. Caller is responsible for sizing
    /// the snippet content within the topbar's height envelope.
    labelSnippet?: Snippet;
    /// Optional corner-overlay slot. Sits in its own stacking
    /// context so `dimmed` doesn't fade the badge along with the
    /// icon. Most callers pass `<AppletBadge ... />` here.
    badge?: Snippet;
    /// Tooltip text. Doubles as the accessible name (`aria-label`)
    /// when no explicit `ariaLabel` is supplied.
    tooltip?: string;
    /// `true` skips the 1500ms delay for tooltips that need to land
    /// instantly (e.g. workspace pills); for topbar applets the
    /// delay is the better default.
    tooltipInstant?: boolean;
    /// `true` while this applet's popover is rendered. Drives the
    /// accent-tinted "open" look and suppresses the tooltip.
    popoverOpen?: boolean;
    /// Semantic underlying status — see the AppletState enum
    /// docstring above. Optional; absence means "neutral / no
    /// state-coloured chrome".
    state?: AppletState;
    /// Render the applet at reduced opacity to signal "I exist
    /// but my underlying subsystem is currently low-priority"
    /// (Network disconnected, BT off, etc.). Badge stays full
    /// opacity.
    dimmed?: boolean;
    /// Non-interactive. Click handlers are not invoked, but the
    /// tooltip still shows so the user knows why.
    disabled?: boolean;
    /// Identifier for keyboard-nav focus-grid. Optional but
    /// recommended for shell-managed applets.
    appletId?: string;
    onclick?: () => void;
    onmouseenter?: () => void;
    oncontextmenu?: () => void;
    /// Wheel handler. Used by Audio for scroll-to-adjust-volume.
    /// Receives the raw WheelEvent so the caller can read deltaY +
    /// preventDefault. Most applets won't need this.
    onWheel?: (e: WheelEvent) => void;
    /// Override for the accessible name when it should differ from
    /// the tooltip text — e.g. a NotificationsTrigger with
    /// "Notifications" tooltip but "3 unread notifications" aria.
    ariaLabel?: string;
  } = $props();

  // Dev-mode validation: every applet must have something for
  // assistive tech to announce. An applet without icon + label +
  // tooltip is a silent button — caught at compile time would be
  // ideal, runtime warning is the next-best safeguard.
  $effect(() => {
    if (!icon && !label && !tooltip) {
      console.warn(
        "[Applet] No icon, label, or tooltip provided — accessible name is empty.",
      );
    }
  });

  const accessibleName = $derived(ariaLabel ?? tooltip ?? label ?? "");
  const showTooltip = $derived(!!tooltip && !popoverOpen);
</script>

{#snippet body()}
  <button
    type="button"
    class="applet"
    class:has-label={!!label || !!labelSnippet}
    class:popover-open={popoverOpen}
    class:dimmed
    class:state-on={state === "on"}
    class:state-off={state === "off"}
    class:state-connecting={state === "connecting"}
    class:state-warn={state === "warn"}
    class:state-error={state === "error"}
    aria-label={accessibleName}
    aria-pressed={popoverOpen}
    data-applet-id={appletId}
    {disabled}
    onclick={() => onclick?.()}
    onmouseenter={() => onmouseenter?.()}
    onwheel={onWheel}
    oncontextmenu={(e) => {
      if (oncontextmenu) {
        e.preventDefault();
        oncontextmenu();
      }
    }}
  >
    <span class="applet-content">
      {#if icon}
        <span class="applet-icon">{@render icon()}</span>
      {/if}
      {#if labelSnippet}
        <span class="applet-label">{@render labelSnippet()}</span>
      {:else if label}
        <span class="applet-label">{label}</span>
      {/if}
    </span>
    {#if badge}
      <span class="applet-badge-slot">{@render badge()}</span>
    {/if}
  </button>
{/snippet}

{#if showTooltip}
  <Tooltip.Root instant={tooltipInstant}>
    <Tooltip.Trigger>
      {@render body()}
    </Tooltip.Trigger>
    <Tooltip.Portal>
      <Tooltip.Content
        class="applet-tooltip"
        side="bottom"
        sideOffset={6}
      >
        {tooltip}
      </Tooltip.Content>
    </Tooltip.Portal>
  </Tooltip.Root>
{:else}
  {@render body()}
{/if}

<style>
  .applet {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    width: var(--topbar-applet-min-w, 28px);
    height: var(--topbar-applet-h, 28px);
    padding: 0;
    background: transparent;
    border: none;
    border-radius: var(--topbar-applet-radius, var(--radius-chip));
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    cursor: pointer;
    transition:
      background-color var(--duration-fast, 100ms) var(--ease-out, ease),
      color var(--duration-fast, 100ms) var(--ease-out, ease),
      opacity var(--duration-fast, 100ms) var(--ease-out, ease),
      transform var(--duration-micro, 60ms) var(--ease-out, ease);
  }
  .applet.has-label {
    /* When a label is present, applet grows beyond the square
       minimum and pads horizontally so icon+label aren't crammed
       at the edges. */
    width: auto;
    min-width: var(--topbar-applet-min-w, 28px);
    padding: 0 8px;
  }

  .applet:hover:not(:disabled) {
    background: var(
      --topbar-applet-hover-bg,
      color-mix(in srgb, var(--color-fg-shell) 10%, transparent)
    );
    color: var(--color-fg-shell);
  }
  .applet:focus-visible {
    outline: none;
    background: var(
      --topbar-applet-hover-bg,
      color-mix(in srgb, var(--color-fg-shell) 10%, transparent)
    );
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 35%, transparent);
  }
  .applet:active:not(:disabled) {
    transform: scale(0.96);
  }
  .applet:disabled {
    cursor: not-allowed;
  }

  /* Popover-open state: accent-coloured icon + subtle bg-tint.
     Kept light (15% accent mix) so it works in monochrome themes
     where accent equals the foreground colour. */
  .applet.popover-open {
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    color: var(--color-accent);
  }
  .applet.popover-open:hover {
    background: color-mix(in srgb, var(--color-accent) 22%, transparent);
  }

  /* Semantic state colours — applied only to the icon, not the bg,
     so they read as foreground accents without competing with the
     popover-open state's bg-tint. */
  .applet.state-on:not(.popover-open) {
    color: var(--color-fg-shell);
  }
  .applet.state-warn:not(.popover-open) {
    color: var(--color-warning, #eab308);
  }
  .applet.state-error:not(.popover-open) {
    color: var(--color-error, #ef4444);
  }
  .applet.state-connecting:not(.popover-open) {
    /* Soft pulse so transient states read without being noisy. */
    animation: applet-pulse 1.6s ease-in-out infinite;
  }
  @keyframes applet-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }

  /* Dimmed state: lowers icon + label opacity but the badge slot
     keeps full opacity (separate stacking context, see below). */
  .applet.dimmed .applet-content {
    opacity: 0.4;
  }

  .applet-content {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .applet-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: var(--topbar-applet-icon-size, 14px);
    height: var(--topbar-applet-icon-size, 14px);
  }

  .applet-label {
    font-size: 0.75rem;
    line-height: 1;
    max-width: var(--topbar-applet-label-max-w, 120px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }

  /* Badge slot: own stacking context (positioned + isolated z) so
     the icon's `dimmed` opacity doesn't fade the badge with it.
     Caller positions the actual badge inside this wrapper (the
     standard AppletBadge sits top-right). */
  .applet-badge-slot {
    position: absolute;
    inset: 0;
    pointer-events: none;
    isolation: isolate;
    z-index: 1;
  }

  /* Tooltip styling — applies through the bits-ui portal. The
     content class lands as `.applet-tooltip` on the portal'd
     div regardless of where it renders in the DOM. */
  :global(.applet-tooltip) {
    background: var(--topbar-tooltip-bg, var(--color-bg-card, #171717));
    color: var(--topbar-tooltip-fg, var(--color-fg-shell));
    font-size: 0.75rem;
    padding: 4px 8px;
    border-radius: var(--radius-chip);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    box-shadow: var(--shadow-md);
    z-index: 200;
    animation: applet-tooltip-in 100ms ease-out both;
  }
  @keyframes applet-tooltip-in {
    from {
      opacity: 0;
      transform: translateY(-2px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
</style>
