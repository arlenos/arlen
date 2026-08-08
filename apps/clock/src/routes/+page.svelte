<script lang="ts">
  /// The clock window (clock-app.md §0): one compact utility, five surfaces
  /// behind a tab row - the task-manager chrome, not a sidebar. The titlebar
  /// carries no bottom border; the tabs row below draws the one hairline.
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Plus } from "lucide-svelte";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import AlarmsView from "$lib/components/AlarmsView.svelte";
  import TimersView from "$lib/components/TimersView.svelte";
  import FocusView from "$lib/components/FocusView.svelte";
  import StopwatchView from "$lib/components/StopwatchView.svelte";
  import WorldView from "$lib/components/WorldView.svelte";
  import { clockMocked, clockUnavailable } from "$lib/stores/clock";
  import { requestAdd } from "$lib/stores/ui";
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
    {#if tab === "alarms" || tab === "world"}
      <button
        type="button"
        class="add-btn"
        id="chrome-add"
        aria-label={tab === "alarms" ? $t("c.al.add") : $t("c.wo.search")}
        onclick={requestAdd}
      >
        <Plus size={16} strokeWidth={2} />
      </button>
    {/if}
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
  {:else if $clockUnavailable}
    <p class="sample">{$t("c.unavailable")}</p>
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
    /* The clock's two-tier type system: one big display per surface, one
       list-time size, everything else the small tier. */
    --clock-display: 2.75rem;
    --clock-list-time: 1.75rem;
  }
  /* The knowledge/store header recipe; no bottom border here - the tabs row
     right below draws the one hairline. */
  .titlebar {
    display: flex;
    align-items: center;
    height: 2.75rem;
    padding: 0 0.35rem 0 1rem;
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
  /* The one fixed add affordance, in the chrome (the macOS pattern). */
  .add-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    margin-inline-end: 2px;
    border: none;
    border-radius: var(--radius-input, 6px);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    cursor: pointer;
  }
  .add-btn:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    color: var(--color-fg-primary);
  }
  /* The GNOME-Clocks grammar: the view switcher sits centered under the
     title, the surfaces below center their columns. */
  .tabs {
    display: flex;
    justify-content: center;
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
    inset-inline: 0.7rem;
    bottom: -1px;
    height: 2px;
    background: var(--color-fg-primary);
  }
  .sample {
    margin: 0.6rem 1rem 0;
    text-align: center;
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
