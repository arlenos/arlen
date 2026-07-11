<script lang="ts">
  /// A flat, dependency-free chart renderer for the closed chart types. Plain
  /// inline SVG scaled from the typed numeric series; monochrome (foreground at
  /// stepped opacities) to match the house style. No charting lib, no executable
  /// spec - the values are just numbers.
  import type { ChartType, Series } from "./types";

  let { chartType, series }: { chartType: ChartType; series: Series[] } = $props();

  // A fixed viewBox; the SVG scales to its container width.
  const W = 320;
  const H = 180;
  const PAD = 24;

  // Series tints: the foreground at stepped opacities, cycled. Flat + monochrome.
  const tint = (i: number): string => {
    const steps = [0.85, 0.55, 0.35, 0.2];
    return `color-mix(in srgb, var(--foreground) ${steps[i % steps.length] * 100}%, transparent)`;
  };

  const maxLen = $derived(Math.max(1, ...series.map((s) => s.values.length)));
  const allValues = $derived(series.flatMap((s) => s.values));
  const vMax = $derived(allValues.length ? Math.max(...allValues, 0) : 1);
  const vMin = $derived(allValues.length ? Math.min(...allValues, 0) : 0);
  const span = $derived(vMax - vMin || 1);

  // Map a data point to SVG coordinates (x by index, y by value).
  const xAt = (i: number): number =>
    maxLen <= 1 ? W / 2 : PAD + (i / (maxLen - 1)) * (W - 2 * PAD);
  const yAt = (v: number): number => H - PAD - ((v - vMin) / span) * (H - 2 * PAD);

  const linePath = (s: Series): string =>
    s.values.map((v, i) => `${i === 0 ? "M" : "L"}${xAt(i)},${yAt(v)}`).join(" ");
  const areaPath = (s: Series): string => {
    if (!s.values.length) return "";
    const top = s.values.map((v, i) => `${i === 0 ? "M" : "L"}${xAt(i)},${yAt(v)}`).join(" ");
    return `${top} L${xAt(s.values.length - 1)},${yAt(vMin)} L${xAt(0)},${yAt(vMin)} Z`;
  };

  // Grouped bars: each index gets one slot, split across series.
  const barW = $derived(
    Math.max(2, ((W - 2 * PAD) / maxLen / Math.max(1, series.length)) * 0.7),
  );

  // Pie from the first series.
  const pie = $derived.by(() => {
    const vals = (series[0]?.values ?? []).map((v) => Math.max(0, v));
    const total = vals.reduce((a, b) => a + b, 0) || 1;
    const cx = W / 2;
    const cy = H / 2;
    const r = Math.min(W, H) / 2 - PAD;
    let a0 = -Math.PI / 2;
    return vals.map((v, i) => {
      const a1 = a0 + (v / total) * Math.PI * 2;
      const x0 = cx + r * Math.cos(a0);
      const y0 = cy + r * Math.sin(a0);
      const x1 = cx + r * Math.cos(a1);
      const y1 = cy + r * Math.sin(a1);
      const large = a1 - a0 > Math.PI ? 1 : 0;
      a0 = a1;
      return { d: `M${cx},${cy} L${x0},${y0} A${r},${r} 0 ${large} 1 ${x1},${y1} Z`, i };
    });
  });
</script>

<svg class="chart" viewBox="0 0 {W} {H}" role="img" aria-label="{chartType} chart" preserveAspectRatio="xMidYMid meet">
  <!-- baseline -->
  {#if chartType !== "pie"}
    <line x1={PAD} y1={yAt(vMin)} x2={W - PAD} y2={yAt(vMin)} class="axis" />
  {/if}

  {#if chartType === "pie"}
    {#each pie as slice (slice.i)}
      <path d={slice.d} fill={tint(slice.i)} stroke="var(--color-bg-app)" stroke-width="1" />
    {/each}
  {:else if chartType === "bar"}
    {#each series as s, si (s.name)}
      {#each s.values as v, i (i)}
        <rect
          x={xAt(i) - (series.length * barW) / 2 + si * barW}
          y={Math.min(yAt(v), yAt(vMin))}
          width={barW}
          height={Math.abs(yAt(v) - yAt(vMin))}
          fill={tint(si)}
        />
      {/each}
    {/each}
  {:else if chartType === "scatter"}
    {#each series as s, si (s.name)}
      {#each s.values as v, i (i)}
        <circle cx={xAt(i)} cy={yAt(v)} r="2.5" fill={tint(si)} />
      {/each}
    {/each}
  {:else}
    {#each series as s, si (s.name)}
      {#if chartType === "area"}
        <path d={areaPath(s)} fill={tint(si)} opacity="0.5" />
      {/if}
      <path d={linePath(s)} fill="none" stroke={tint(si)} stroke-width="1.5" />
    {/each}
  {/if}
</svg>

{#if series.length > 1}
  <div class="legend">
    {#each series as s, si (s.name)}
      <span class="legend-item"><span class="swatch" style="background:{tint(si)}"></span>{s.name}</span>
    {/each}
  </div>
{/if}

<style>
  .chart {
    width: 100%;
    height: auto;
    display: block;
  }
  .axis {
    stroke: color-mix(in srgb, var(--foreground) 15%, transparent);
    stroke-width: 1;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.875rem;
    margin-top: 0.5rem;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
  }
  .swatch {
    width: 0.625rem;
    height: 0.625rem;
    border-radius: var(--radius-chip);
    flex-shrink: 0;
  }
</style>
