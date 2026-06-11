<script lang="ts">
  /// Inline CSD window buttons (minimize / maximize / close) for an app's
  /// own titlebar, used when the window runs with `decorations: false`.
  /// This is the in-header button cluster an app places at the right edge
  /// of its slim CSD header; the full-width shell-overlay variant with its
  /// own drag region is `WindowControls.svelte`.
  import { Minus, Square, X } from "@lucide/svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  let {
    showMinimize = true,
    showMaximize = true,
    class: className = "",
  }: {
    /// Hide minimize where it makes no sense (e.g. the compositor reports
    /// the window as tiled; minimizing a tiled surface breaks the layout).
    showMinimize?: boolean;
    /// Hide maximize for fixed-size windows.
    showMaximize?: boolean;
    /// Additional CSS classes for the root element.
    class?: string;
  } = $props();

  async function minimize() {
    await getCurrentWindow().minimize();
  }
  async function toggleMaximize() {
    const w = getCurrentWindow();
    if (await w.isMaximized()) {
      await w.unmaximize();
    } else {
      await w.maximize();
    }
  }
  async function close() {
    await getCurrentWindow().close();
  }
</script>

<div class="window-buttons {className}">
  {#if showMinimize}
    <button
      type="button"
      class="wb-btn"
      onclick={minimize}
      aria-label="Minimize"
    >
      <Minus size={12} strokeWidth={2} />
    </button>
  {/if}
  {#if showMaximize}
    <button
      type="button"
      class="wb-btn"
      onclick={toggleMaximize}
      aria-label="Maximize"
    >
      <Square size={10} strokeWidth={2} />
    </button>
  {/if}
  <button
    type="button"
    class="wb-btn wb-close"
    onclick={close}
    aria-label="Close"
  >
    <X size={12} strokeWidth={2} />
  </button>
</div>

<style>
  .window-buttons {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .wb-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    border: none;
    background: transparent;
    color: var(--color-fg-secondary);
    border-radius: var(--radius-input);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .wb-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }
  /* Close gets the error tint on hover (the banner pattern), not a solid
     fill, so the cluster stays calm and no literal color is needed. */
  .wb-close:hover {
    background: color-mix(in srgb, var(--color-error) 15%, transparent);
    color: var(--color-error);
  }
</style>
