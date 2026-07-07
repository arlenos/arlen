<script lang="ts">
  /// The system monitor window: a five-tab dashboard. Overview is the built default
  /// (the sovereign glance); the other four are legible in the tab strip but their
  /// content is a later phase - shown honestly, never faked.
  import { onMount } from "svelte";
  import OverviewTab from "$lib/components/monitor/OverviewTab.svelte";
  import { load } from "$lib/stores/overview";

  const TABS = ["Overview", "Apps", "Access", "Devices", "System"] as const;
  let tab = $state<(typeof TABS)[number]>("Overview");

  onMount(load);
</script>

<div class="app">
  <header class="titlebar">
    <span class="app-title">System monitor</span>
  </header>

  <nav class="tabs" aria-label="Views">
    {#each TABS as t (t)}
      <button type="button" class="tab" class:active={tab === t} onclick={() => (tab = t)}>
        {t}
      </button>
    {/each}
  </nav>

  <main class="content">
    {#if tab === "Overview"}
      <OverviewTab />
    {:else}
      <p class="placeholder">This view is coming.</p>
    {/if}
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    min-height: 100vh;
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
  .content {
    flex: 1;
    padding: 1.75rem 1.25rem;
  }
  .placeholder {
    margin: 3rem 0 0;
    font-size: 0.9375rem;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
</style>
