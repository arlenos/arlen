<script lang="ts">
  /// Timers (clock-app.md §0.2): presets + a custom duration, running timers
  /// as rows. Remaining time derives from the daemon's `ends_at` anchor and
  /// the render tick - the window never counts.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { clock, tick, startTimer, pauseTimer, cancelTimer, timerRemaining } from "$lib/stores/clock";
  import { fmtDuration } from "$lib/format";
  import { t } from "$lib/i18n/messages";

  const PRESETS = [1, 5, 10, 25];
  let customMin = $state(15);
  let customSec = $state(0);
  const durationMs = $derived((customMin * 60 + customSec) * 1000);
</script>

<div class="ti">
  <!-- One instrument, one action: the big readout IS the duration being set,
       the presets and steppers set it, Start starts it. -->
  <div class="ti-new">
    <span class="ti-readout" aria-hidden="true">{fmtDuration(durationMs)}</span>
    <div class="ti-instrument">
      <div class="ti-steppers">
        <NumberInput value={customMin} min={0} max={999} width="100%" unit={$t("c.ti.minutes")} ariaLabel={$t("c.ti.minutes")} onchange={(v) => (customMin = v)} />
        <NumberInput value={customSec} min={0} max={59} width="100%" unit={$t("c.ti.seconds")} ariaLabel={$t("c.ti.seconds")} onchange={(v) => (customSec = v)} />
      </div>
      <div class="ti-presets">
        {#each PRESETS as p (p)}
          <Button
            variant="outline"
            size="sm"
            id={`preset-${p}`}
            onclick={() => {
              customMin = p;
              customSec = 0;
            }}
          >
            {$t("c.ti.min", { n: p })}
          </Button>
        {/each}
      </div>
      <Button id="start-timer" class="ti-start" disabled={durationMs === 0} onclick={() => startTimer(durationMs)}>
        {$t("c.ti.start")}
      </Button>
    </div>
  </div>

  {#if $clock}
    {#if $clock.timers.length === 0}
      <p class="ti-empty">{$t("c.ti.none")}</p>
    {:else}
      <div class="ti-list">
        {#each $clock.timers as ti (ti.id)}
          {@const remaining = timerRemaining(ti, $tick)}
          {@const pct = ti.duration_ms > 0 ? (remaining / ti.duration_ms) * 100 : 0}
          <div class="ti-row">
            <span class="ti-ring" style={`--p:${pct}`} aria-hidden="true"></span>
            <span class="ti-remaining" class:paused={ti.paused}>{fmtDuration(remaining)}</span>
            <span class="ti-of">/ {fmtDuration(ti.duration_ms)}</span>
            <span class="ti-spacer"></span>
            <Button variant="outline" size="sm" onclick={() => pauseTimer(ti.id, !ti.paused)}>
              {ti.paused ? $t("c.ti.resume") : $t("c.ti.pause")}
            </Button>
            <Button variant="ghost" size="sm" class="text-muted-foreground" onclick={() => cancelTimer(ti.id)}>
              {$t("c.ti.cancel")}
            </Button>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .ti {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-width: 30rem;
    margin: 0 auto;
    padding: 1.1rem 1rem 1.5rem;
  }
  .ti-new {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    padding-top: 0.75rem;
  }
  .ti-readout {
    font-size: var(--clock-display, 2.75rem);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    color: var(--color-fg-primary);
  }
  /* One instrument width: the steppers, the presets and Start all share it,
     so every row ends on the same edges (rule 5). */
  .ti-instrument {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    width: 16.5rem;
  }
  .ti-steppers {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem;
  }
  .ti-presets {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0.4rem;
  }
  .ti-presets :global(button) {
    width: 100%;
  }
  .ti-instrument :global(.ti-start) {
    width: 100%;
  }
  .ti-empty {
    margin: 0;
    font-size: var(--text-sm);
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .ti-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .ti-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.8rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  /* The thin progress ring around nothing but geometry (rule 8): a conic
     fill masked to a 3px annulus. */
  .ti-ring {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    border-radius: var(--radius-full, 9999px);
    background: conic-gradient(
      var(--color-fg-primary) calc(var(--p) * 1%),
      color-mix(in srgb, var(--color-fg-primary) 15%, transparent) 0
    );
    -webkit-mask: radial-gradient(farthest-side, transparent calc(100% - 3px), #000 calc(100% - 3px + 0.5px));
    mask: radial-gradient(farthest-side, transparent calc(100% - 3px), #000 calc(100% - 3px + 0.5px));
  }
  .ti-remaining {
    font-size: var(--clock-list-time, 1.75rem);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }
  .ti-remaining.paused {
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .ti-of {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .ti-spacer {
    flex: 1;
  }
</style>
