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
    transition: opacity var(--duration-fast) var(--easing-default);
  }

  .window-buttons :global(.control-btn:hover) {
    opacity: 1;
  }

  .window-buttons :global(.close-btn:hover) {
    background-color: var(--destructive);
    color: #ffffff;
  }
</style>
