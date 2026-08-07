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
    <div class="ti-steppers">
      <NumberInput value={customMin} min={0} max={999} width="120px" unit={$t("c.ti.minutes")} ariaLabel={$t("c.ti.minutes")} onchange={(v) => (customMin = v)} />
      <NumberInput value={customSec} min={0} max={59} width="120px" unit={$t("c.ti.seconds")} ariaLabel={$t("c.ti.seconds")} onchange={(v) => (customSec = v)} />
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
    <Button id="start-timer" disabled={durationMs === 0} onclick={() => startTimer(durationMs)}>
      {$t("c.ti.start")}
    </Button>
  </div>

  {#if $clock}
    {#if $clock.timers.length === 0}
      <p class="ti-empty">{$t("c.ti.none")}</p>
    {:else}
      <div class="ti-list">
        {#each $clock.timers as ti (ti.id)}
          {@const remaining = timerRemaining(ti, $tick)}
          <div class="ti-row">
            <span class="ti-remaining" class:paused={ti.paused}>{fmtDuration(remaining)}</span>
            <span class="ti-of">/ {fmtDuration(ti.duration_ms)}</span>
            <span class="ti-state">{ti.paused ? $t("c.ti.paused") : $t("c.ti.running")}</span>
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
    font-size: 2.6rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    color: var(--color-fg-primary);
  }
  .ti-steppers {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .ti-presets {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 0.4rem;
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
  .ti-remaining {
    font-size: var(--text-xl);
    font-weight: 500;
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
  .ti-state {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .ti-spacer {
    flex: 1;
  }
</style>
