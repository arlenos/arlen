<script lang="ts">
  /// The task-manager window. The landing IS the process list (no verdict page).
  /// Tabs sit above it (Processes first, always); a toolbar carries the filter + the
  /// group/flatten toggle.
  import { onMount } from "svelte";
  import ProcessTable from "$lib/components/tm/ProcessTable.svelte";
  import PerformanceTab from "$lib/components/tm/PerformanceTab.svelte";
  import DetailPane from "$lib/components/tm/DetailPane.svelte";
  import RowMenu from "$lib/components/tm/RowMenu.svelte";
  import { processes, mocked, lastError, load, stop, pause, resume, limit, unlimit, type Process } from "$lib/stores/processes";
  import { startPerf, stopPerf } from "$lib/stores/perf";
  import { t, dir } from "$lib/i18n/messages";
  import { Rows3, Layers, Search } from "lucide-svelte";

  const TABS = [
    { key: "Processes", id: "tm.tab.processes" },
    { key: "Performance", id: "tm.tab.performance" },
  ] as const;
  let tab = $state<(typeof TABS)[number]["key"]>("Processes");
  let filter = $state("");
  let flatten = $state(false);
  let selected = $state<Process | null>(null);
  let menu = $state<{ proc: Process; x: number; y: number } | null>(null);

  onMount(load);

  // Run the ~1 Hz Performance ticks only while that tab is visible.
  $effect(() => {
    if (tab === "Performance") startPerf();
    else stopPerf();
    return stopPerf;
  });
</script>

<div class="app" dir={$dir}>
  <header class="titlebar">
    <span class="app-title">{$t("tm.title")}</span>
  </header>

  <nav class="tabs" aria-label="Views">
    {#each TABS as tb (tb.key)}
      <button type="button" class="tab" class:active={tab === tb.key} onclick={() => (tab = tb.key)}>
        {$t(tb.id)}
      </button>
    {/each}
  </nav>

  {#if tab === "Processes"}
    {#if $mocked}
      <!-- Every row here offers a Stop; unlabelled, the fixture reads as this
           machine's real processes. -->
      <p class="note">{$t("tm.sample")}</p>
    {/if}
    {#if $lastError}
      <!-- A refused action must be visible: the row already reverted, and this
           says why, so a failed Stop never passes as a killed process. -->
      <p class="note error" role="alert">{$lastError}</p>
    {/if}
    <div class="toolbar">
      <span class="filter">
        <Search size={14} strokeWidth={2} class="filter-icon" />
        <input
          class="filter-input"
          placeholder={$t("tm.filter.placeholder")}
          bind:value={filter}
          aria-label={$t("tm.filter.aria")}
        />
      </span>
      <span class="spacer"></span>
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
</div>

{#if menu}
  <RowMenu
    process={menu.proc}
    x={menu.x}
    y={menu.y}
    onStop={stop}
    onForceQuit={(id) => {
      stop(id);
      if (selected?.id === id) selected = null;
    }}
    onDetails={(p) => (selected = p)}
    onPause={pause}
    onResume={resume}
    onLimit={limit}
    onUnlimit={unlimit}
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
  .titlebar {
    display: flex;
    align-items: center;
    height: 2.5rem;
    padding: 0 1.25rem;
    flex-shrink: 0;
  }
  .app-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
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
    padding: 0.6rem 0.75rem;
    border: none;
    background: transparent;
    font-size: var(--text-base);
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
    inset-inline: 0.75rem;
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
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.7rem 1rem;
    flex-shrink: 0;
  }
  .filter {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    width: 16rem;
    padding: 0.35rem 0.6rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input, 8px);
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .filter :global(.filter-icon) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .filter-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--color-fg-primary);
    font-size: var(--text-sm);
    outline: none;
  }
  .filter-input::placeholder {
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
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
