<script lang="ts">
  /// The task-manager window. The landing IS the process list (no verdict page).
  /// Tabs sit above it (Processes first, always); a toolbar carries the filter + the
  /// group/flatten toggle.
  import { onMount } from "svelte";
  import ProcessTable from "$lib/components/tm/ProcessTable.svelte";
  import PerformanceTab from "$lib/components/tm/PerformanceTab.svelte";
  import DetailPane from "$lib/components/tm/DetailPane.svelte";
  import RowMenu from "$lib/components/tm/RowMenu.svelte";
  import { processes, mocked, unavailable, lastError, load, startProcessPolling, stopProcessPolling, stop, stopRow, pause, resume, limit, unlimit, pauseRow, resumeRow, limitRow, unlimitRow, type Process } from "$lib/stores/processes";
  import { startPerf, stopPerf } from "$lib/stores/perf";
  import { t, dir } from "$lib/i18n/messages";
  import { Rows3, Layers } from "lucide-svelte";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { RATES, rateLabel, refreshMs, setRefreshMs } from "$lib/refresh";

  const TABS = [
    { key: "Processes", id: "tm.tab.processes" },
    { key: "Performance", id: "tm.tab.performance" },
  ] as const;
  let tab = $state<(typeof TABS)[number]["key"]>("Processes");
  let filter = $state("");
  let flatten = $state(false);
  let selected = $state<Process | null>(null);
  let menu = $state<{ proc: Process; x: number; y: number } | null>(null);

  /// A lever pressed on a row applies to the whole row.
  ///
  /// The menu hands back a pid because that is what a row carries; an app row's
  /// pid is only its eldest member, so acting on it alone leaves the rest of the
  /// app running under a label that says otherwise.
  function byRow(
    id: number,
    onRow: (p: Process) => Promise<void>,
    onPid: (id: number) => Promise<void>,
  ): Promise<void> {
    const row = $processes.find((p) => p.id === id);
    return row ? onRow(row) : onPid(id);
  }

  onMount(load);

  // Run the ~1 Hz Performance ticks only while that tab is visible.
  $effect(() => {
    if (tab === "Performance") startPerf();
    else stopPerf();
    return stopPerf;
  });

  // Keep the process list refreshing while it is on screen. The backend derives
  // CPU% and disk rates from the delta against the previous sample, so a single
  // load at mount left every row reading 0.0% forever.
  $effect(() => {
    if (tab === "Processes") startProcessPolling();
    else stopProcessPolling();
    return stopProcessPolling;
  });

  // Window chrome: explicit startDragging (data-tauri-drag-region is
  // unreliable on Wayland in Tauri v2), guarded so vite still renders.
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

<!-- `main`, not `div`: this is the page's content, and a document with no main
     landmark leaves a screen-reader user no way to skip the chrome. -->
<main class="app" dir={$dir}>
  <!-- The header is a drag surface (a non-keyboard pointer interaction); its
       actual controls are the accessible WindowButtons inside it, so the
       static-interaction lint is a false positive here. Same treatment as the
       knowledge and store headers. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="titlebar" onpointerdown={startDrag} ondblclick={toggleMax}>
    <span class="app-title">{$t("tm.title")}</span>
    <span class="titlebar-spacer"></span>
    <WindowButtons />
  </header>

  <nav class="tabs" aria-label={$t("tm.views")}>
    {#each TABS as tb (tb.key)}
      <!-- Addressable like the clock's tabs (`id="tab-alarms"`). Performance
           lives behind this click, so with no way to press it the panel
           carrying the 8 August memory bug had never been in a headless
           no-backend shot. -->
      <button
        type="button"
        class="tab"
        id={`tab-${tb.key.toLowerCase()}`}
        class:active={tab === tb.key}
        onclick={() => (tab = tb.key)}
      >
        {$t(tb.id)}
      </button>
    {/each}
  </nav>

  {#if tab === "Processes"}
    {#if $mocked}
      <!-- Every row here offers a Stop; unlabelled, the fixture reads as this
           machine's real processes. -->
      <p class="note">{$t("tm.sample")}</p>
    {:else if $unavailable}
      <!-- No rows at all rather than a labelled fixture: the ids in that fixture
           are the argument Stop passes to the backend. -->
      <p class="note">{$t("tm.unavailable")}</p>
    {/if}
    {#if $lastError}
      <!-- A refused action must be visible: the row already reverted, and this
           says why, so a failed Stop never passes as a killed process. -->
      <p class="note error" role="alert">{$lastError}</p>
    {/if}
    <div class="toolbar">
      <span class="filter">
        <SearchField
          placeholder={$t("tm.filter.placeholder")}
          bind:value={filter}
          aria-label={$t("tm.filter.aria")}
        />
      </span>
      <span class="spacer"></span>
      <!-- The global refresh rate (system-monitor-plan.md (a)). Both pollers
           read it, so the Processes list and the Performance tab agree on how
           current their numbers are; before this they ran at 2 Hz and 1 Hz from
           two independent timers with no control over either. -->
      <label class="rate">
        <span class="rate-label">{$t("tm.rate.label")}</span>
        <select
          class="rate-select"
          aria-label={$t("tm.rate.aria")}
          value={String($refreshMs)}
          onchange={(e) => setRefreshMs(Number((e.currentTarget as HTMLSelectElement).value))}
        >
          {#each RATES as r (r)}
            <option value={String(r)}>{rateLabel(r)}</option>
          {/each}
        </select>
      </label>
      <button
        type="button"
        class="toggle"
        class:on={flatten}
        title={$t(flatten ? "tm.toggle.toGrouped" : "tm.toggle.toAll")}
        onclick={() => (flatten = !flatten)}
      >
        {#if flatten}<Rows3 size={14} strokeWidth={2} /> {$t("tm.toggle.all")}{:else}<Layers size={14} strokeWidth={2} /> {$t("tm.toggle.grouped")}{/if}
      </button>
    </div>

    <div class="proc-body">
      <div class="table-wrap">
        <ProcessTable
          list={$processes}
          {filter}
          {flatten}
          selectedId={selected?.id}
          onSelect={(p) => (selected = p)}
          onContextMenu={(p, x, y) => (menu = { proc: p, x, y })}
        />
      </div>
      {#if selected}
        <DetailPane
          process={selected}
          onClose={() => (selected = null)}
          onForceQuit={(id) => {
            stop(id);
            selected = null;
          }}
        />
      {/if}
    </div>
  {:else}
    <div class="perf-wrap">
      <PerformanceTab />
    </div>
  {/if}
</main>

{#if menu}
  <RowMenu
    process={menu.proc}
    x={menu.x}
    y={menu.y}
    onStop={(id) => {
      // The ROW, not the pid: an app row stands for its whole group and the
      // plan says Stop takes the tree.
      const row = $processes.find((p) => p.id === id);
      return row ? stopRow(row) : stop(id);
    }}
    onForceQuit={(id) => {
      stop(id);
      if (selected?.id === id) selected = null;
    }}
    onDetails={(p) => (selected = p)}
    onPause={(id) => byRow(id, pauseRow, pause)}
    onResume={(id) => byRow(id, resumeRow, resume)}
    onLimit={(id) => byRow(id, limitRow, limit)}
    onUnlimit={(id) => byRow(id, unlimitRow, unlimit)}
    onClose={() => {
      const pid = menu?.proc.id;
      menu = null;
      if (pid != null)
        requestAnimationFrame(() =>
          (document.querySelector(`.row[data-pid="${pid}"]`) as HTMLElement | null)?.focus(),
        );
    }}
  />
{/if}

<svelte:body oncontextmenu={(e) => e.preventDefault()} />

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  /* The knowledge/store header recipe: 2.75rem, title, spacer, window
     controls, drag region. No bottom border here - the tabs nav right below
     already draws the one hairline, and two rules 2.1rem apart read as a
     mistake. */
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
  .tabs {
    display: flex;
    gap: 0.25rem;
    /* The first tab's TEXT sits on the content edge: the nav start padding is
       the content inset minus the tab's own inline padding. */
    padding: 0 1rem 0 calc(1rem - 0.7rem);
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    flex-shrink: 0;
  }
  .tab {
    position: relative;
    padding: 0.5rem 0.7rem;
    border: none;
    background: transparent;
    /* One step below the bar title: the tabs are navigation, not identity. */
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    cursor: pointer;
  }
  .tab:hover {
    color: var(--color-fg-primary);
  }
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
  /* Calm caveat above the table - it qualifies every row below it. */
  .note {
    margin: 0;
    padding: 0.6rem 1rem 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    flex-shrink: 0;
  }
  /* A refused action is the one thing here worth a colour. */
  .note.error {
    color: var(--color-fg-danger, #f87171);
  }
  .rate {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .rate-select {
    background: var(--color-bg-input, #1f1f1f);
    color: var(--color-fg-primary, #fafafa);
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: 6px;
    padding: 3px 6px;
    font: inherit;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.7rem 1rem;
    flex-shrink: 0;
  }
  .filter {
    width: 16rem;
  }
  .spacer {
    flex: 1;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.7rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    cursor: pointer;
  }
  .toggle:hover {
    color: var(--color-fg-primary);
  }
  .proc-body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .table-wrap {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 0 0.4rem;
  }
  .perf-wrap {
    flex: 1;
    min-height: 0;
  }
</style>
