<script lang="ts">
  /// The row right-click menu (the home for Stop now that the per-row button is
  /// gone): a small popup at the cursor with the process actions. A backdrop or
  /// Escape dismisses it.
  import { trapFocus } from "@arlen/ui-kit/keyboard/trap_focus";
  import { t } from "$lib/i18n/messages";
  import { niceLevels, niceOf, niceUnavailable, renice, type NiceLevel, type Process } from "$lib/stores/processes";

  /// The Advanced affordance (system-monitor-plan.md (c)): priority behind a
  /// disclosure rather than beside Stop, because a nice value is expert
  /// vocabulary and the direction of it is famously backwards.
  ///
  /// Real-time scheduling is deliberately NOT here. The plan says to warn
  /// against it, and not shipping the control is the strongest warning: a
  /// SCHED_FIFO runaway can need the power button.
  let showAdvanced = $state(false);
  let levels = $state<NiceLevel[]>([]);
  let current = $state<number | null>(null);

  $effect(() => {
    if (!showAdvanced) return;
    const pid = process.id;
    void niceLevels().then((l) => (levels = l));
    void niceOf(pid).then((n) => {
      if (process.id === pid) current = n;
    });
  });

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
    <button
      type="button"
      class="mi"
      role="menuitem"
      aria-expanded={showAdvanced}
      onclick={(e) => {
        e.stopPropagation();
        showAdvanced = !showAdvanced;
      }}
    >
      {$t("tm.menu.advanced")}
    </button>
    {#if showAdvanced}
      <div class="mi-adv">
        <span class="mi-cap">{$t("tm.menu.priority")}</span>
        {#each levels as l (l.nice)}
          <button
            type="button"
            class="mi mi-sub"
            class:on={current === l.nice}
            role="menuitem"
            onclick={() => {
              void renice(process.id, l.nice);
              onClose();
            }}
          >
            {$t(l.key)}
          </button>
        {/each}
        {#if $niceUnavailable}
          <span class="mi-cap">{$t("tm.menu.priorityUnavailable")}</span>
        {/if}
      </div>
    {/if}
    <div class="mi-sep" role="separator"></div>
    <!-- Stop asks nothing: the process gets to save, and launching it again
         undoes it. The two exceptions ask ONCE in the page's dialog, which names
         the loss: a system service (the row carries `critical` from the core's
         own name list) because nothing here starts it again, and Force quit
         because it ends the process without letting it save. -->
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
    font-size: var(--text-2xs);
    color: var(--color-fg-secondary, #a1a1aa);
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
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
    text-align: start;
  }
  .mi:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .mi-adv {
    display: flex;
    flex-direction: column;
    padding: 2px 0 4px;
  }
  .mi-cap {
    padding: 4px 12px 2px;
    font-size: 11px;
    color: var(--color-fg-disabled, #737373);
  }
  .mi-sub {
    /* Logical, not physical: in an RTL locale these items hang off the right
       edge, where a padding-left would indent them away from their parent. */
    padding-inline-start: 22px;
  }
  .mi-sub.on::before {
    content: "\2713\00a0";
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
