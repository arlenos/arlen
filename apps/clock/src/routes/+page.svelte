<script lang="ts">
  /// The clock window (clock-app.md §0): one compact utility, five surfaces
  /// behind a tab row - the task-manager chrome, not a sidebar. The titlebar
  /// carries no bottom border; the tabs row below draws the one hairline.
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import AlarmsView from "$lib/components/AlarmsView.svelte";
  import TimersView from "$lib/components/TimersView.svelte";
  import FocusView from "$lib/components/FocusView.svelte";
  import StopwatchView from "$lib/components/StopwatchView.svelte";
  import WorldView from "$lib/components/WorldView.svelte";
  import { clockMocked } from "$lib/stores/clock";
  import { t, dir } from "$lib/i18n/messages";

  const TABS = [
    { key: "alarms", id: "c.tab.alarms" },
    { key: "timers", id: "c.tab.timers" },
    { key: "focus", id: "c.tab.focus" },
    { key: "stopwatch", id: "c.tab.stopwatch" },
    { key: "world", id: "c.tab.world" },
  ] as const;
  type TabKey = (typeof TABS)[number]["key"];
  let tab = $state<TabKey>("alarms");

  // Window chrome: explicit startDragging (the drag attribute is unreliable on
  // Wayland in Tauri v2), guarded so vite still renders.
  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }
  async function startDrag(e: PointerEvent): Promise<void> {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      // No Tauri runtime under vite: the header is a static bar.
    }
  }
  async function toggleMax(e: MouseEvent): Promise<void> {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      // Same guard as above.
    }
  }
</script>

<div class="app" dir={$dir}>
  <!-- The header is a drag surface (a non-keyboard pointer interaction); its
       actual controls are the accessible WindowButtons inside it, so the
       static-interaction lint is a false positive here. Same treatment as the
       knowledge and store headers. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="titlebar" onpointerdown={startDrag} ondblclick={toggleMax}>
    <span class="app-title">{$t("c.title")}</span>
    <span class="titlebar-spacer"></span>
    <WindowButtons />
  </header>

  <nav class="tabs" aria-label={$t("c.tabs")}>
    {#each TABS as tb (tb.key)}
      <button type="button" class="tab" class:active={tab === tb.key} id={`tab-${tb.key}`} onclick={() => (tab = tb.key)}>
        {$t(tb.id)}
      </button>
    {/each}
  </nav>

  {#if $clockMocked}
    <p class="sample">{$t("c.sample")}</p>
  {/if}

  <main class="body">
    {#if tab === "alarms"}
      <AlarmsView />
    {:else if tab === "timers"}
      <TimersView />
    {:else if tab === "focus"}
      <FocusView />
    {:else if tab === "stopwatch"}
      <StopwatchView />
    {:else}
      <WorldView />
    {/if}
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  /* The knowledge/store header recipe; no bottom border here - the tabs row
     right below draws the one hairline. */
  .titlebar {
    display: flex;
    align-items: center;
    height: 2.75rem;
    padding: 0 0.35rem 0 0.9rem;
    flex-shrink: 0;
    user-select: none;
    -webkit-user-select: none;
  }
  .app-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .titlebar-spacer {
    flex: 1;
  }
  .tabs {
    display: flex;
    gap: 0.25rem;
    padding: 0 1rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    flex-shrink: 0;
  }
  .tab {
    position: relative;
    padding: 0.5rem 0.7rem;
    border: none;
    background: transparent;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    cursor: pointer;
  }
  .tab:hover,
  .tab.active {
    color: var(--color-fg-primary);
  }
  .tab.active::after {
    content: "";
    position: absolute;
    inset-inline: 0.75rem;
    bottom: -1px;
    height: 2px;
    background: var(--color-fg-primary);
  }
  .sample {
    margin: 0.6rem 1rem 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    flex-shrink: 0;
  }
  .body {
    min-height: 0;
    flex: 1;
    overflow-y: auto;
  }
</style>
