<script lang="ts">
  /// The row right-click menu (the home for Stop now that the per-row button is
  /// gone): a small popup at the cursor with the process actions. A backdrop or
  /// Escape dismisses it.
  import { trapFocus } from "@arlen/ui-kit/keyboard/trap_focus";
  import { t } from "$lib/i18n/messages";
  import type { Process } from "$lib/stores/processes";

  let {
    process,
    x,
    y,
    onStop,
    onForceQuit,
    onDetails,
    onPause,
    onResume,
    onLimit,
    onUnlimit,
    onClose,
  }: {
    process: Process;
    x: number;
    y: number;
    onStop: (id: number) => void;
    onForceQuit: (id: number) => void;
    onDetails: (p: Process) => void;
    onPause: (id: number) => void;
    onResume: (id: number) => void;
    onLimit: (id: number) => void;
    onUnlimit: (id: number) => void;
    onClose: () => void;
  } = $props();

  // Keep the menu on screen.
  const left = $derived(Math.min(x, (typeof window !== "undefined" ? window.innerWidth : 1280) - 190));
  const top = $derived(Math.min(y, (typeof window !== "undefined" ? window.innerHeight : 800) - 140));

  let menuEl = $state<HTMLElement | null>(null);

  function menuItems(): HTMLElement[] {
    return menuEl ? [...menuEl.querySelectorAll<HTMLElement>('[role="menuitem"]')] : [];
  }
  function menuKeydown(e: KeyboardEvent) {
    const list = menuItems();
    if (list.length === 0) return;
    const cur = list.indexOf(document.activeElement as HTMLElement);
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        list[(cur + 1 + list.length) % list.length].focus();
        break;
      case "ArrowUp":
        e.preventDefault();
        list[(cur - 1 + list.length) % list.length].focus();
        break;
      case "Home":
        e.preventDefault();
        list[0].focus();
        break;
      case "End":
        e.preventDefault();
        list[list.length - 1].focus();
        break;
      case "Escape":
        e.preventDefault();
        onClose();
        break;
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape") onClose();
  }}
/>

<div
  class="backdrop"
  role="presentation"
  onclick={onClose}
  oncontextmenu={(e) => {
    e.preventDefault();
    onClose();
  }}
>
  <div
    class="menu"
    style="left: {left}px; top: {top}px"
    role="menu"
    aria-label={process.name}
    tabindex="-1"
    bind:this={menuEl}
    onkeydown={menuKeydown}
    use:trapFocus={{ returnFocus: false }}
  >
    <div class="menu-head">{process.name}</div>
    <button type="button" class="mi" role="menuitem" onclick={() => { onDetails(process); onClose(); }}>
      {$t("tm.menu.details")}
    </button>
    {#if process.paused}
      <button type="button" class="mi" role="menuitem" onclick={() => { onResume(process.id); onClose(); }}>
        {$t("tm.menu.resume")}
      </button>
    {:else}
      <button type="button" class="mi" role="menuitem" onclick={() => { onPause(process.id); onClose(); }}>
        {$t("tm.menu.pause")}
      </button>
    {/if}
    {#if process.limited}
      <button type="button" class="mi" role="menuitem" onclick={() => { onUnlimit(process.id); onClose(); }}>
        {$t("tm.menu.unlimit")}
      </button>
    {:else}
      <button type="button" class="mi" role="menuitem" onclick={() => { onLimit(process.id); onClose(); }}>
        {$t("tm.menu.limit")}
      </button>
    {/if}
    <div class="mi-sep" role="separator"></div>
    <button type="button" class="mi" role="menuitem" onclick={() => { onStop(process.id); onClose(); }}>
      {$t("tm.menu.stop")}
    </button>
    <button type="button" class="mi danger" role="menuitem" onclick={() => { onForceQuit(process.id); onClose(); }}>
      {$t("tm.menu.forceQuit")}
    </button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
  }
  .menu {
    position: fixed;
    min-width: 11rem;
    padding: 0.25rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    border-radius: var(--radius-input, 8px);
    background: var(--color-bg-card, #171717);
    box-shadow: var(--shadow-lg, 0 8px 30px #00000066);
  }
  .menu-head {
    padding: 0.35rem 0.55rem 0.4rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mi {
    display: block;
    width: 100%;
    padding: 0.4rem 0.55rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font-size: 0.8125rem;
    color: var(--color-fg-primary);
    text-align: start;
    cursor: pointer;
  }
  .mi:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .mi-sep {
    height: 1px;
    margin: 0.25rem 0.4rem;
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .mi.danger {
    color: var(--color-error, #c96a6a);
  }
  .mi.danger:hover {
    background: color-mix(in srgb, var(--color-error, #c96a6a) 14%, transparent);
  }
</style>
