<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The Performance tab (Windows-Performance shape): a device list on the left
  /// (name + current value + a mini live sparkline), the selected device's big live
  /// graph + its current figures on the right.
  import Graph from "./Graph.svelte";
  import { series, tick, perfError, axisMax, DEVICES, type Device } from "$lib/stores/perf";

  let selected = $state<Device>("cpu");
  const sel = $derived(DEVICES.find((d) => d.key === selected) ?? DEVICES[0]);

  const n = (v: number, digits = 0) => v.toFixed(digits);

  /// The headline figure per device, from the last tick. A device with nothing
  /// measured shows a dash: a zero here would be a claim that the machine is idle,
  /// which is not the same as not having asked it.
  function value(d: Device): string {
    const s = $tick;
    if (!s) return "\u2014";
    switch (d) {
      // CPU is a rate like disk and network: on the first tick there is nothing
      // to subtract from, and printing the 0 the host returns would state that
      // the machine is idle when nobody has measured it yet.
      case "cpu":
        return s.ratesReady ? `${n(s.cpuPct)}%` : "\u2014";
      // Memory is a level, so it is real from the first tick.
      case "memory":
        return `${n(s.memPct)}%`;
      case "disk":
        return s.ratesReady ? `${n(s.diskReadMbs + s.diskWriteMbs, 1)} MB/s` : "\u2014";
      case "network":
        return s.ratesReady ? `${n(s.netRxMbs + s.netTxMbs, 1)} MB/s` : "\u2014";
      case "ai":
        return "\u2014";
    }
  }

  /// The line under the big graph.
  function detail(d: Device): string {
    const s = $tick;
    if (d === "ai") return $t("tm.perf.ai.detail");
    if (!s) return $t("tm.perf.waiting");
    switch (d) {
      case "cpu":
        return $t("tm.perf.cpu.detail", { count: String(s.cpuCount) });
      case "memory":
        return $t("tm.perf.mem.detail", {
          used: n(s.memUsedGb, 1),
          total: n(s.memTotalGb, 1),
        });
      case "disk":
        return s.ratesReady
          ? $t("tm.perf.disk.detail", { read: n(s.diskReadMbs, 1), write: n(s.diskWriteMbs, 1) })
          : $t("tm.perf.waiting");
      case "network":
        return s.ratesReady
          ? $t("tm.perf.net.detail", { down: n(s.netRxMbs, 1), up: n(s.netTxMbs, 1) })
          : $t("tm.perf.waiting");
    }
  }
</script>

<!-- The figures below are read from /proc and /sys by the host, once a second.
     The label that used to sit here saying they were examples went in the same
     commit that made them real - an exception outlives its reason otherwise. The
     one thing still unmeasured is the AI device, which says so where it renders
     rather than drawing a line. -->
{#if $perfError}
  <p class="perf-sample" role="alert">{$t("tm.perf.unavailable")}</p>
{/if}

<div class="perf">
  <div class="devices" role="tablist" aria-label={$t("tm.perf.devices")}>
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
          <span class="dev-name">{$t(d.label)}</span>
          <span class="dev-val">{value(d.key)}</span>
        </div>
        <div class="dev-spark">
          <Graph series={$series[d.key]} max={axisMax($series[d.key], d.max)} variant="spark" />
        </div>
      </button>
    {/each}
  </div>

  <div class="main">
    <div class="main-head">
      <h2 class="main-title">{$t(sel.label)}</h2>
      <span class="main-val">{value(selected)}</span>
    </div>
    <div class="main-graph">
      <Graph series={$series[selected]} max={axisMax($series[selected], sel.max)} variant="big" />
    </div>
    <div class="main-detail">{detail(selected)}</div>
  </div>
</div>

<style>
  .perf-sample {
    margin: 0;
    padding: 8px 12px 0;
    font-size: 12px;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

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
