<script lang="ts">
  /// Stopwatch (clock-app.md §0.4): anchors + daemon pause snapshots, the
  /// window only renders. Laps list quiet, newest on top, with deltas.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { clock, tick, stopwatchStart, stopwatchPause, stopwatchLap, stopwatchReset, stopwatchTotal } from "$lib/stores/clock";
  import { fmtDuration, fmtStopwatch } from "$lib/format";
  import { t } from "$lib/i18n/messages";
</script>

<div class="sw">
  {#if $clock}
    {@const swx = $clock.stopwatch}
    {@const total = stopwatchTotal(swx, $tick)}
    {@const cs = String(Math.floor((Math.max(0, total) % 1000) / 10)).padStart(2, "0")}
    <span class="sw-total" class:idle={!swx.running && total === 0}>
      {fmtDuration(total)}<span class="sw-cs">.{cs}</span>
    </span>
    <!-- Equal twins that relabel in place and never move (rule 4): the left
         slot is Lap/Reset, the right slot Start/Stop. -->
    <div class="sw-twins">
      {#if swx.running}
        <Button variant="outline" size="lg" class="sw-twin" id="sw-lap" onclick={stopwatchLap}>{$t("c.sw.lap")}</Button>
        <Button size="lg" class="sw-twin" id="sw-stop" onclick={stopwatchPause}>{$t("c.sw.pause")}</Button>
      {:else}
        <Button
          variant="outline"
          size="lg"
          class="sw-twin"
          id="sw-reset"
          disabled={total === 0}
          onclick={stopwatchReset}
        >
          {$t("c.sw.reset")}
        </Button>
        <Button size="lg" class="sw-twin" id="sw-start" onclick={stopwatchStart}>
          {total === 0 ? $t("c.sw.start") : $t("c.sw.resume")}
        </Button>
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
    max-width: 30rem;
    margin: 0 auto;
    padding: 2rem 1rem 1.5rem;
  }
  .sw-total {
    font-size: var(--clock-display, 2.75rem);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    color: var(--color-fg-primary);
  }
  .sw-cs {
    font-size: 0.55em;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .sw-total.idle {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  /* The twins span the column together, half each - same width discipline
     as every tile below them. */
  .sw-twins {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.75rem;
    align-self: stretch;
  }
  .sw-twins :global(.sw-twin) {
    width: 100%;
  }
  .sw-laps {
    display: flex;
    flex-direction: column;
    align-self: stretch;
    margin-top: 0.5rem;
    padding: 0.25rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .sw-lap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 7rem;
    align-items: baseline;
    column-gap: 0.75rem;
    padding: 0.5rem 0;
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
