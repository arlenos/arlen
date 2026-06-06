<script lang="ts">
  /// QS tile: Knowledge Graph activity sparkline.
  ///
  /// 1×1 tile next to the Project tile. Renders a small inline SVG
  /// area+line sparkline of the last 8 days of event counts. The
  /// chart is gated behind `hasData` — when the daemon reports zero
  /// events in the window the SVG is not rendered at all (no flat
  /// baseline that would read as an oversized divider). Real data
  /// with low variation still draws as a small modulated line, which
  /// is honest about the underlying activity.
  ///
  /// No strip / no statusText — the tile is purely informational and
  /// clicks straight into Settings → Knowledge.
  import { BaseTile } from "@arlen/ui-kit/components/quicksettings";
  import { Brain } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  interface Bucket {
    day: number;
    count: number;
  }
  interface Stats {
    available: boolean;
    buckets: Bucket[];
    today: number;
    total: number;
  }

  let stats = $state<Stats>({
    available: false,
    buckets: [],
    today: 0,
    total: 0,
  });

  /// KG entries land via the event-bus pipeline so freshness is
  /// bounded by the writer's batch cadence (500ms / 1000 events).
  /// 60s is well within "feels live" without hammering the daemon.
  const POLL_MS = 60_000;

  onMount(() => {
    refresh();
    const interval = setInterval(refresh, POLL_MS);
    return () => clearInterval(interval);
  });

  async function refresh() {
    try {
      stats = await invoke<Stats>("knowledge_daily_counts");
    } catch {
      stats = { available: false, buckets: [], today: 0, total: 0 };
    }
  }

  function openSettings() {
    invoke("quick_action_run", { id: "qa.open_settings_knowledge" }).catch(
      () => {},
    );
  }

  const SVG_W = 100;
  const SVG_H = 24;

  /// Render the chart only when there is at least one event in the
  /// 8-day window. An all-zero series would draw a flat baseline
  /// that visually reads as an oversized divider strip.
  const hasData = $derived(
    stats.available && stats.buckets.some((b) => b.count > 0),
  );

  /// Empty-state message for the body region when there's no chart
  /// to render. Distinguishes daemon-offline from "daemon up but
  /// nothing recorded yet" so the user knows whether to debug or
  /// just wait.
  const emptyMessage = $derived(
    !stats.available ? "Daemon offline" : "No events yet",
  );

  const linePath = $derived.by(() => {
    if (!hasData) return "";
    const max = Math.max(1, ...stats.buckets.map((b) => b.count));
    const stepX = SVG_W / Math.max(1, stats.buckets.length - 1);
    return stats.buckets
      .map((b, i) => {
        const x = i * stepX;
        const y = SVG_H - (b.count / max) * (SVG_H - 4) - 2;
        return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
  });

  /// Closed-area path for the fill: line plus baseline corners.
  const areaPath = $derived(
    hasData ? `${linePath} L${SVG_W},${SVG_H} L0,${SVG_H} Z` : "",
  );
</script>

<div class="kg-tile-wrap">
  <BaseTile label="Knowledge" active={hasData} onclick={openSettings}>
    {#snippet icon()}
      <Brain size={16} strokeWidth={1.75} />
    {/snippet}
    {#if hasData}
      <div class="kg-chart">
        <svg
          viewBox="0 0 {SVG_W} {SVG_H}"
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          <path class="kg-spark-area" d={areaPath} />
          <path class="kg-spark-line" d={linePath} />
        </svg>
      </div>
    {:else}
      <div class="kg-empty">{emptyMessage}</div>
    {/if}
  </BaseTile>
</div>

<style>
  .kg-tile-wrap {
    width: 100%;
  }
  .kg-tile-wrap :global(.qs-tile) {
    width: 100%;
  }

  /* Chart container — rendered as the BaseTile children snippet,
     so it lands in `.qs-tile-body` below the head. Padding pulls
     the line in from the tile edges so it doesn't kiss the rounded
     corners. */
  .kg-chart {
    padding: 4px 12px 12px 12px;
    height: 28px;
    width: 100%;
    box-sizing: border-box;
  }
  .kg-chart svg {
    width: 100%;
    height: 100%;
    display: block;
  }
  .kg-spark-line {
    fill: none;
    stroke: var(--color-accent);
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .kg-spark-area {
    fill: color-mix(in srgb, var(--color-accent) 18%, transparent);
    stroke: none;
  }

  /* Empty-state body — same vertical footprint as the chart so the
     tile height stays constant whether or not data is showing. */
  .kg-empty {
    padding: 4px 12px 12px 12px;
    height: 28px;
    width: 100%;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    line-height: 1.2;
  }
</style>
