<script lang="ts">
  /// The task-manager window. The landing IS the process list (no verdict page).
  /// Tabs sit above it (Processes first, always); a toolbar carries the filter + the
  /// group/flatten toggle.
  import { onMount } from "svelte";
  import ProcessTable from "$lib/components/tm/ProcessTable.svelte";
  import PerformanceTab from "$lib/components/tm/PerformanceTab.svelte";
  import DetailPane from "$lib/components/tm/DetailPane.svelte";
  import { processes, load, stop, type Process } from "$lib/stores/processes";
  import { startPerf, stopPerf } from "$lib/stores/perf";
  import { Rows3, Layers, Search } from "lucide-svelte";

  const TABS = ["Processes", "Performance"] as const;
  let tab = $state<(typeof TABS)[number]>("Processes");
  let filter = $state("");
  let flatten = $state(false);
  let selected = $state<Process | null>(null);

  onMount(load);

  // Run the ~1 Hz Performance ticks only while that tab is visible.
  $effect(() => {
    if (tab === "Performance") startPerf();
    else stopPerf();
    return stopPerf;
  });
</script>

<div class="app">
  <header class="titlebar">
    <span class="app-title">Task manager</span>
  </header>

  <nav class="tabs" aria-label="Views">
    {#each TABS as t (t)}
      <button type="button" class="tab" class:active={tab === t} onclick={() => (tab = t)}>{t}</button>
    {/each}
  </nav>

  {#if tab === "Processes"}
    <div class="toolbar">
      <span class="filter">
        <Search size={14} strokeWidth={2} class="filter-icon" />
        <input class="filter-input" placeholder="Filter" bind:value={filter} aria-label="Filter processes" />
      </span>
      <span class="spacer"></span>
      <button
        type="button"
        class="toggle"
        class:on={flatten}
        title={flatten ? "Group by app" : "Show every process"}
        onclick={() => (flatten = !flatten)}
      >
        {#if flatten}<Rows3 size={14} strokeWidth={2} /> All processes{:else}<Layers size={14} strokeWidth={2} /> Grouped{/if}
      </button>
    </div>

    <div class="proc-body">
      <div class="table-wrap">
        <ProcessTable
          list={$processes}
          {filter}
          {flatten}
          selectedId={selected?.id}
          onStop={stop}
          onSelect={(p) => (selected = p)}
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
    font-size: 0.8125rem;
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
    font-size: 0.875rem;
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
    left: 0.75rem;
    right: 0.75rem;
    bottom: -1px;
    height: 2px;
    background: var(--color-fg-primary);
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
    font-size: 0.8125rem;
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
    font-size: 0.8125rem;
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
