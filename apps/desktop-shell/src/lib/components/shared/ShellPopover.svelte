<script lang="ts">
  /// The one popover surface every topbar applet opens: full-screen
  /// click-away backdrop + a right-anchored panel hanging below the
  /// bar. Consumers provide the anchor offset and width (the only
  /// two things that ever differed between the hand-rolled copies),
  /// an optional PopoverHeader via the `header` snippet, and the
  /// body content.
  ///
  /// Open state is driven exclusively by the shared `activePopover`
  /// store — its helpers own the `set_popover_input_region` call
  /// that makes the compositor route clicks to this layer, so the
  /// base never flips visibility on its own. Escape is handled
  /// globally in +layout.svelte (Escape closes whatever popover is
  /// active); the base deliberately adds no second handler.
  ///
  /// `keepMounted` renders the panel permanently and toggles it via
  /// a visibility class instead of {#if}: the notifications surface
  /// keeps its store subscriptions alive across opens (no re-fetch)
  /// and gets a fade-out for free. Default popovers mount fresh per
  /// open — their `$effect`-on-open data loads depend on that.

  import type { Snippet } from "svelte";
  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import type { PopoverType } from "$lib/stores/activePopover.js";

  let {
    id,
    width,
    right,
    maxHeight,
    keepMounted = false,
    bodyPadding,
    bodyGap,
    header,
    children,
  }: {
    /// The `activePopover` id this surface answers to.
    id: PopoverType;
    /// Panel width in px.
    width: number;
    /// Anchor offset from the screen's right edge in px. Hand-tuned
    /// per applet so the panel hangs under its trigger.
    right: number;
    /// Optional cap, e.g. "calc(100vh - 56px)" — adds a scroll
    /// region on the panel itself. Most popovers cap inner lists
    /// instead and leave this unset.
    maxHeight?: string;
    /// Permanently mount the panel and toggle visibility by class.
    keepMounted?: boolean;
    /// Body padding override (default 8px). Settings-style bodies
    /// use 12px.
    bodyPadding?: string;
    /// Body row gap override (default 2px for hover-row lists).
    bodyGap?: string;
    /// Rendered flush at the top of the panel — a PopoverHeader.
    header?: Snippet;
    /// Body content; lands inside the padded `.pop-body` column.
    children: Snippet;
  } = $props();

  const open = $derived($activePopover === id);
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={() => closePopover()}></div>
{/if}

{#if keepMounted || open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="pop-panel shell-popover"
    class:pop-panel-lazy={!keepMounted}
    class:pop-panel-pinned={keepMounted}
    class:pop-panel-visible={keepMounted && open}
    style:right="{right}px"
    style:width="{width}px"
    style:--pop-body-padding={bodyPadding}
    style:--pop-body-gap={bodyGap}
    style:max-height={maxHeight}
    style:overflow-y={maxHeight ? "auto" : undefined}
    onclick={(e) => e.stopPropagation()}
  >
    {#if header}
      {@render header()}
    {/if}
    <div class="pop-body">
      {@render children()}
    </div>
  </div>
{/if}

<style>
  /* z 90/100: the backdrop/panel pair of the topbar popover layer —
     see the z-scale table in app.css. */
  .pop-backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
  }

  .pop-panel {
    position: fixed;
    top: 40px;
    z-index: 100;
    border-radius: var(--radius-card);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    display: flex;
    flex-direction: column;
    transform-origin: top center;
  }

  /* Fresh-mount mode: entry animation only — the {#if} unmount is
     instant by design. Keyframes live in sdk/ui-kit/src/lib/motion.css. */
  .pop-panel-lazy {
    animation: arlen-popover-in var(--duration-medium) var(--ease-out) both;
  }

  /* Pinned mode: the panel stays in the DOM and fades in place.
     The `visibility` transition delays hiding until the fade ends
     on close, and snaps to visible instantly on open. */
  .pop-panel-pinned {
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    transform: translateY(-4px) scale(0.98);
    transition:
      opacity var(--duration-medium) var(--ease-out),
      transform var(--duration-medium) var(--ease-out),
      visibility 0s linear var(--duration-medium);
  }
  .pop-panel-visible {
    opacity: 1;
    visibility: visible;
    pointer-events: auto;
    transform: translateY(0) scale(1);
    transition:
      opacity var(--duration-medium) var(--ease-out),
      transform var(--duration-medium) var(--ease-out),
      visibility 0s linear 0s;
  }

  .pop-body {
    padding: var(--pop-body-padding, 8px);
    display: flex;
    flex-direction: column;
    gap: var(--pop-body-gap, 2px);
    min-height: 0;
  }
</style>
