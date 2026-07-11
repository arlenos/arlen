<script lang="ts">
  /// The Performance tab (Windows-Performance shape): a device list on the left
  /// (name + current value + a mini live sparkline), the selected device's big live
  /// graph + its current figures on the right.
  import Graph from "./Graph.svelte";
  import { series, stats, DEVICES, type Device } from "$lib/stores/perf";

  let selected = $state<Device>("cpu");
  const sel = $derived(DEVICES.find((d) => d.key === selected) ?? DEVICES[0]);
</script>

<div class="perf">
  <div class="devices" role="tablist" aria-label="Devices">
    {#each DEVICES as d (d.key)}
      <button
        type="button"
        class="dev"
        class:active={selected === d.key}
        role="tab"
        aria-selected={selected === d.key}
        onclick={() => (selected = d.key)}
      >
        <div class="dev-info">
          <span class="dev-name">{d.label}</span>
          <span class="dev-val">{$stats[d.key].value}</span>
        </div>
        <div class="dev-spark">
          <Graph series={$series[d.key]} max={d.max} variant="spark" />
        </div>
      </button>
    {/each}
  </div>

  <div class="main">
    <div class="main-head">
      <h2 class="main-title">{sel.label}</h2>
      <span class="main-val">{$stats[selected].value}</span>
    </div>
    <div class="main-graph">
      <Graph series={$series[selected]} max={sel.max} variant="big" />
    </div>
    <div class="main-detail">{$stats[selected].detail}</div>
  </div>
</div>

<style>
  .perf {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .devices {
    width: 15rem;
    flex-shrink: 0;
    padding: 0.5rem;
    border-inline-end: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-y: auto;
  }
  .dev {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
    padding: 0.6rem 0.7rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    cursor: pointer;
    text-align: start;
  }
  .dev:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .dev.active {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .dev-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .dev-name {
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
  }
  .dev-val {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .dev-spark {
    width: 4.5rem;
    height: 2rem;
    flex-shrink: 0;
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 1.5rem 1.75rem;
  }
  .main-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 1rem;
  }
  .main-title {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .main-val {
    font-size: 1.35rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }
  .main-graph {
    flex: 1;
    min-height: 0;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card, 12px);
    overflow: hidden;
  }
  .main-detail {
    margin-top: 0.9rem;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
</style>
