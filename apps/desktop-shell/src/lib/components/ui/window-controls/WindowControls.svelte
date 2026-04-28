<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Minus, Square, X } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button/index.js";

  interface Props {
    /** Whether to show the maximize button. False for fixed-size windows. */
    showMaximize?: boolean;
    /** Additional CSS classes for the root element. */
    class?: string;
  }

  let { showMaximize = true, class: className = "" }: Props = $props();

  const win = getCurrentWindow();

  async function minimize() {
    await win.minimize();
  }

  async function maximize() {
    const maximized = await win.isMaximized();
    if (maximized) {
      await win.unmaximize();
    } else {
      await win.maximize();
    }
  }

  async function close() {
    await win.close();
  }
</script>

<!--
  WindowControls: Client-Side Decoration window controls for Lunaris.

  Used by desktop-shell to render window controls as an overlay.
  NOT imported by individual apps directly.

  The drag region covers the full bar. Users drag the window by clicking
  anywhere that is not a button.
-->
<div
  class="lunaris-window-controls shell-surface {className}"
  data-tauri-drag-region
>
  <div class="drag-region" data-tauri-drag-region></div>

  <div class="window-buttons">
    <Button
      variant="ghost"
      size="icon"
      class="control-btn"
      onclick={minimize}
      aria-label="Minimize"
    >
      <Minus size={12} strokeWidth={2} />
    </Button>

    {#if showMaximize}
      <Button
        variant="ghost"
        size="icon"
        class="control-btn"
        onclick={maximize}
        aria-label="Maximize"
      >
        <Square size={10} strokeWidth={2} />
      </Button>
    {/if}

    <Button
      variant="ghost"
      size="icon"
      class="control-btn close-btn"
      onclick={close}
      aria-label="Close"
    >
      <X size={12} strokeWidth={2} />
    </Button>
  </div>
</div>

<style>
  .lunaris-window-controls {
    display: flex;
    align-items: center;
    height: 36px;
    width: 100%;
    position: relative;
    user-select: none;
    flex-shrink: 0;
  }

  .drag-region {
    position: absolute;
    inset: 0;
    z-index: 0;
  }

  .window-buttons {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
    padding-right: 6px;
    z-index: 1;
  }

  .window-buttons :global(.control-btn) {
    width: 28px;
    height: 22px;
    padding: 0;
    opacity: 0.7;
    /*
      Transform + background transitions on `--duration-micro` match
      the rest of the shell's interactive-chrome feel (see
      .interactive in sdk/ui-kit/motion.css). Opacity uses
      `--duration-fast` because a slightly longer fade reads as
      deliberate rather than jumpy. Baseline `scale(1)` is required
      so the hover/active scales have a GPU-composited layer ready
      from the first frame — without it the first scale triggers
      a repaint.
    */
    transform: scale(1);
    transition:
      opacity var(--duration-fast, 150ms) var(--ease-out, ease-out),
      transform var(--duration-micro, 100ms) var(--ease-out, ease-out),
      background-color var(--duration-micro, 100ms) var(--ease-out, ease-out),
      color var(--duration-micro, 100ms) var(--ease-out, ease-out);
  }

  .window-buttons :global(.control-btn:hover) {
    opacity: 1;
    transform: scale(1.1);
    /* `--foreground` is not defined in the desktop-shell theme
       scope (it's a shadcn token from ui-kit). Use the shell's
       canonical fg token so the hover rectangle actually shows up
       against the topbar / titlebar background. */
    background-color: color-mix(in srgb, var(--color-fg-shell, currentColor) 10%, transparent);
  }

  .window-buttons :global(.control-btn:active) {
    transform: scale(0.9);
  }

  .window-buttons :global(.control-btn:focus-visible) {
    outline: 2px solid var(--color-accent, currentColor);
    outline-offset: 1px;
  }

  .window-buttons :global(.close-btn:hover) {
    background-color: var(--destructive);
    color: #ffffff;
  }

  /*
    Reduced-motion guardrail: motion.css zeroes the duration tokens
    under `prefers-reduced-motion: reduce`, but the scale transforms
    still apply instantly without a transition. Disable them here so
    the button stays visually static under that preference.
  */
  @media (prefers-reduced-motion: reduce) {
    .window-buttons :global(.control-btn),
    .window-buttons :global(.control-btn:hover),
    .window-buttons :global(.control-btn:active) {
      transform: none;
    }
  }
</style>
