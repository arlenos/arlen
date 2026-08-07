<script lang="ts">
  /// Stopwatch (clock-app.md §0.4): anchors + daemon pause snapshots, the
  /// window only renders. Laps list quiet, newest on top, with deltas.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { clock, tick, stopwatchStart, stopwatchPause, stopwatchLap, stopwatchReset, stopwatchTotal } from "$lib/stores/clock";
  import { fmtStopwatch } from "$lib/format";
  import { t } from "$lib/i18n/messages";
</script>

<div class="sw">
  {#if $clock}
    {@const swx = $clock.stopwatch}
    {@const total = stopwatchTotal(swx, $tick)}
    <span class="sw-total" class:idle={!swx.running && total === 0}>{fmtStopwatch(total)}</span>
    <div class="sw-actions">
      {#if swx.running}
        <Button variant="outline" id="sw-pause" onclick={stopwatchPause}>{$t("c.sw.pause")}</Button>
        <Button id="sw-lap" onclick={stopwatchLap}>{$t("c.sw.lap")}</Button>
      {:else}
        <Button id="sw-start" onclick={stopwatchStart}>{total === 0 ? $t("c.sw.start") : $t("c.sw.resume")}</Button>
        {#if total > 0}
          <Button variant="ghost" id="sw-reset" class="text-muted-foreground" onclick={stopwatchReset}>{$t("c.sw.reset")}</Button>
        {/if}
      {/if}
    </div>

    {#if swx.laps.length > 0}
      <div class="sw-laps">
        {#each [...swx.laps].reverse() as lap, i (swx.laps.length - i)}
          {@const n = swx.laps.length - i}
          {@const prev = n > 1 ? swx.laps[n - 2] : 0}
          <div class="sw-lap">
            <span class="sw-lap-n">{$t("c.sw.lapN", { n })}</span>
            <span class="sw-lap-delta">+{fmtStopwatch(lap - prev)}</span>
            <span class="sw-lap-total">{fmtStopwatch(lap)}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .sw {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    max-width: 34rem;
    padding: 2rem 1rem 1.5rem;
  }
  .sw-total {
    font-size: 3.2rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    color: var(--color-fg-primary);
  }
  .sw-total.idle {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .sw-actions {
    display: flex;
    gap: 0.5rem;
  }
  .sw-laps {
    display: flex;
    flex-direction: column;
    align-self: stretch;
    margin-top: 0.5rem;
  }
  .sw-lap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 7rem;
    align-items: baseline;
    column-gap: 0.75rem;
    padding: 0.45rem 0;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .sw-lap:last-child {
    border-bottom: none;
  }
  .sw-lap-n {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .sw-lap-delta {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .sw-lap-total {
    justify-self: end;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }
</style>
