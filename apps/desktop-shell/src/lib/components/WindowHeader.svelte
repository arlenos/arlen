<script lang="ts">
    import {
        windowHeaders,
        draggingSurfaces,
        headerAction,
        HEADER_ACTION_MINIMIZE,
        HEADER_ACTION_MAXIMIZE,
        HEADER_ACTION_CLOSE,
    } from "$lib/stores/windowHeaders";
    import { tabBars, activateTab } from "$lib/stores/tabBars";
    import { Minus, Square, X } from "lucide-svelte";

    /// Look up the tabs (if any) for a given header. Stacks own
    /// tabs via the parallel `tabBars` store, keyed on `stack_id`.
    /// Returns an empty list for non-stack headers so the template
    /// can branch cleanly.
    function tabsFor(stackId: number) {
        if (stackId === 0) return [];
        const bar = $tabBars.get(stackId);
        return bar ? bar.tabs.map((t) => ({
            index: t.index,
            title: t.title,
            active: t.index === bar.active,
            stackId,
        })) : [];
    }
</script>

<!--
  The compositor rasterises no header itself; the shell draws every
  eligible window header here. A plain window (`stack_id === 0`)
  shows its title in the drag area; a stack (`stack_id !== 0`) shows
  the integrated tab strip (Feature 3). Either way the window
  controls sit on the right. The compositor gates emission on
  `should_emit_shell_header_events`, which now mirrors the
  render-eligibility policy, so this `#each` receives one entry per
  window with a header.
-->
{#each [...$windowHeaders.values()] as hdr (hdr.surface_id)}
    {@const tabs = tabsFor(hdr.stack_id)}
    <!--
      `transform: translate3d()` rather than `left/top` so position
      updates are GPU-composited. With separate clients (shell vs.
      window) the one-frame geometry lag between a compositor resize
      and the header's repaint is inevitable; at least the paint
      itself shouldn't invalidate layout each tick.
    -->
    <!--
      `.shell-surface` scope flips `--background`, `--foreground`,
      `--muted-foreground`, and `--border` to shell-chrome values
      (see `app.css:95`). Without it this stack header picked up
      the root `--background = --color-bg-app = #0f0f0f`, which is
      visibly lighter than the `.shell-surface` topbar at
      `--color-bg-shell = #0a0a0a`. With it, stack headers match
      the topbar AND the compositor-rendered single-window headers
      (which now read `theme.bg_shell` for the same reason).
    -->
    <div
        class="window-header shell-surface"
        class:activated={hdr.activated}
        class:dragging={$draggingSurfaces.has(hdr.surface_id)}
        style="
            transform: translate3d({hdr.x}px, {hdr.y}px, 0);
            width: {hdr.width}px;
            height: {hdr.height}px;
        "
    >
        <!--
          Title / drag area. Deliberately NO `onmousedown` handler.
          The compositor routes pointer events inside the top 36px
          SSD zone to its own `PointerTarget::WindowUI` which starts
          interactive move with a real (not synthesized) serial —
          the only way Smithay's move grab actually sticks. The
          shell's input-region is trimmed to just the button
          rectangles (see `update_window_header_regions`), so a
          mousedown on this title area falls through the layer
          surface and reaches the compositor.

          `pointer-events: none` on this element reinforces the
          same contract visually (GTK's hit-testing within the
          layer surface also skips it).
        -->
            <!--
              Integrated stack header (Feature 3). Tabs sit on the
              LEFT as clickable buttons; the drag zone between tabs
              and buttons stays fall-through so the native compositor
              SSD routing still handles move/resize. The input-
              region in windowHeaders.ts covers both the tab strip
              and the button strip.
            -->
        {#if hdr.stack_id !== 0}
            <div class="header-tabs" role="tablist" aria-label="Window tabs">
                {#each tabs as tab (tab.index)}
                    <button
                        type="button"
                        role="tab"
                        class="header-tab"
                        class:header-tab-active={tab.active}
                        onclick={() => activateTab(tab.stackId, tab.index)}
                        aria-selected={tab.active}
                    >
                        <span class="header-tab-title" title={tab.title}>{tab.title}</span>
                    </button>
                {/each}
            </div>
            <div class="header-drag header-drag-grow"></div>
        {:else}
            <!--
              Plain single-window header: the window title fills the
              drag area. `pointer-events: none` (inherited from
              `.header-drag`) keeps a title-bar drag falling through to
              the compositor's native SSD move routing.
            -->
            <div class="header-drag">
                <span class="header-title" title={hdr.title}>{hdr.title}</span>
            </div>
        {/if}

        <div class="header-buttons">
            {#if hdr.has_minimize}
                <button
                    class="header-btn minimize"
                    onclick={() => headerAction(hdr.surface_id, HEADER_ACTION_MINIMIZE)}
                    aria-label="Minimize"
                >
                    <Minus size={14} strokeWidth={2} />
                </button>
            {/if}
            {#if hdr.has_maximize}
                <button
                    class="header-btn maximize"
                    onclick={() => headerAction(hdr.surface_id, HEADER_ACTION_MAXIMIZE)}
                    aria-label="Maximize"
                >
                    <Square size={12} strokeWidth={2} />
                </button>
            {/if}
            <button
                class="header-btn close"
                onclick={() => headerAction(hdr.surface_id, HEADER_ACTION_CLOSE)}
                aria-label="Close"
            >
                <X size={14} strokeWidth={2} />
            </button>
        </div>
    </div>
{/each}

<style>
    .window-header {
        /* translate3d() drives position — see component comment on
           why we avoid left/top. `position: fixed` anchors against
           the viewport so the transform values map 1:1 to compositor
           coordinates; `top: 0; left: 0` is the reference origin. */
        position: fixed;
        top: 0;
        left: 0;
        /* Sits just above the TopBar (z=auto / 0) so a maximized
           window's header can't be eclipsed by the bar, but well
           below every shell overlay (popover backdrop=90, panel=100,
           workspace-map=120, context-menu=300, drag-ghost=10001,
           toast=sonner-default). A Kitty-style ServerSide app can
           span the whole width of the screen, and at z=7000 its
           Arlen header used to paint over shell popovers that
           intersected its geometry — which looked, from the user's
           perspective, like the bar itself was in front of the
           popover. 50 keeps SSD headers visible normally and makes
           them recede behind shell chrome the moment a popover,
           the workspace map, or a context menu opens. */
        z-index: 50;
        display: flex;
        align-items: center;
        background: var(--background);
        color: var(--muted-foreground);
        border-bottom: 1px solid var(--border);
        border-radius: var(--radius-input) var(--radius-input) 0 0;
        overflow: hidden;
        pointer-events: auto;
        will-change: transform;
    }

    .window-header.activated {
        color: var(--foreground);
    }

    /*
      While a drag/resize grab is active the compositor pushes
      `window_header_update` events at pointer-motion rate. Any CSS
      transition on `transform` would lag a few frames behind and
      make the header visibly trail the window. Drop transitions
      for the duration of the drag — `draggingSurfaces` toggles
      this class via the Feature-4 drag_start/drag_end events.
    */
    /* The two-class selector (0,2,0) outranks the base rule's
       (0,1,0) on its own. */
    .window-header.dragging {
        transition: none;
    }

    .header-drag {
        flex: 1;
        min-width: 0;
        display: flex;
        align-items: center;
        padding: 0 12px;
        height: 100%;
        user-select: none;
        -webkit-user-select: none;
        /* pointer-events: none so the layer-surface hit-test never
           counts the title area. Combined with the trimmed input-
           region (button rects only), clicks on this region reach
           the compositor which starts interactive move via its
           built-in SSD-zone PointerTarget::WindowUI routing. */
        pointer-events: none;
    }

    /* Integrated stack header (Feature 3). */
    .header-tabs {
        display: flex;
        align-items: stretch;
        height: 100%;
        padding-left: 8px;
        gap: 2px;
        overflow: hidden;
        max-width: 60%;
        pointer-events: auto;
    }
    .header-tab {
        min-width: 80px;
        max-width: 200px;
        padding: 0 10px;
        display: flex;
        align-items: center;
        border: none;
        background: transparent;
        color: inherit;
        border-top-left-radius: var(--radius-input);
        border-top-right-radius: var(--radius-input);
        font-size: 12px;
        line-height: 1;
        transition:
            background var(--duration-micro, 100ms) var(--ease-out, ease-out),
            color var(--duration-micro, 100ms) var(--ease-out, ease-out);
    }
    .header-tab:hover {
        background: color-mix(in srgb, var(--foreground) 8%, transparent);
    }
    .header-tab-active {
        background: color-mix(in srgb, var(--foreground) 14%, transparent);
        color: var(--foreground);
    }
    .header-tab-title {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        flex: 1;
    }
    /* The remaining drag strip between tabs and buttons. Grow to
       fill, keep pointer-events: none so drag passes through. */
    .header-drag-grow {
        flex: 1;
        min-width: 24px;
        height: 100%;
        user-select: none;
        -webkit-user-select: none;
        pointer-events: none;
    }

    /* Plain single-window title, filling the drag area. */
    .header-title {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: 13px;
        line-height: 1;
    }

    .header-buttons {
        display: flex;
        align-items: center;
        gap: 0;
        padding-right: 4px;
    }

    .header-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: var(--height-control, 28px);
        height: var(--height-control, 28px);
        border: none;
        border-radius: var(--radius-input);
        background: transparent;
        color: inherit;
        font-size: 14px;
        /*
          Micro-duration transforms + fast background/color fades.
          Matches the feel of .control-btn in WindowControls.svelte
          so native Arlen windows and compositor-decorated windows
          present identical interaction feedback. Baseline
          `scale(1)` primes the GPU layer.
        */
        transform: scale(1);
        transition:
            background var(--duration-micro, 100ms) var(--ease-out, ease-out),
            color var(--duration-micro, 100ms) var(--ease-out, ease-out),
            transform var(--duration-micro, 100ms) var(--ease-out, ease-out);
    }

    .header-btn:hover {
        background: color-mix(in srgb, var(--foreground) 10%, transparent);
        transform: scale(1.1);
    }

    .header-btn:active {
        transform: scale(0.9);
    }

    .header-btn:focus-visible {
        outline: 2px solid var(--color-accent, currentColor);
        outline-offset: 1px;
    }

    .header-btn.close:hover {
        background: color-mix(in srgb, var(--color-error) 80%, transparent);
        color: var(--color-fg-primary);
    }

    /*
      `@media (prefers-reduced-motion: reduce)` in sdk/ui-kit/motion.css
      zeroes the duration tokens, which stops the transitions but not
      the transforms themselves — a hover would still snap to 1.1×
      instantly. Disable transforms under that preference so the
      reduce-motion experience is a pure colour swap, no scale pop.
    */
    @media (prefers-reduced-motion: reduce) {
        .header-btn,
        .header-btn:hover,
        .header-btn:active {
            transform: none;
        }
    }
</style>
