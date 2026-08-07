<script lang="ts">
  /// Timers (clock-app.md §0.2): presets + a custom duration, running timers
  /// as rows. Remaining time derives from the daemon's `ends_at` anchor and
  /// the render tick - the window never counts.
  import { Minus, Plus } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { clock, tick, startTimer, pauseTimer, cancelTimer, timerRemaining } from "$lib/stores/clock";
  import { fmtDuration } from "$lib/format";
  import { t } from "$lib/i18n/messages";

  let customMin = $state(15);
  let customSec = $state(0);
  const durationMs = $derived((customMin * 60 + customSec) * 1000);

  function step(minutes: number): void {
    customMin = Math.min(999, Math.max(0, customMin + minutes));
  }
  function clampMin(v: string): void {
    customMin = Math.min(999, Math.max(0, Math.floor(Number(v) || 0)));
  }
  function clampSec(v: string): void {
    customSec = Math.min(59, Math.max(0, Math.floor(Number(v) || 0)));
  }
</script>

<div class="ti">
  <!-- The big display IS the input: minus, editable digits, plus - one
       instrument, no duplication. The buttons step whole minutes; the digits
       take typing directly. -->
  <div class="ti-new">
    <div class="ti-set">
      <Button variant="outline" size="icon" aria-label={$t("c.ti.less")} onclick={() => step(-1)}>
        <Minus size={18} strokeWidth={2} />
      </Button>
      <span class="ti-digits">
        <input
          class="ti-digit min"
          type="text"
          inputmode="numeric"
          value={String(customMin).padStart(2, "0")}
          aria-label={$t("c.ti.minutes")}
          onchange={(e: Event) => clampMin((e.currentTarget as HTMLInputElement).value)}
        />
        <span class="ti-colon">:</span>
        <input
          class="ti-digit sec"
          type="text"
          inputmode="numeric"
          value={String(customSec).padStart(2, "0")}
          aria-label={$t("c.ti.seconds")}
          onchange={(e: Event) => clampSec((e.currentTarget as HTMLInputElement).value)}
        />
      </span>
      <Button variant="outline" size="icon" aria-label={$t("c.ti.more")} onclick={() => step(1)}>
        <Plus size={18} strokeWidth={2} />
      </Button>
    </div>
    <Button id="start-timer" size="lg" class="ti-start" disabled={durationMs === 0} onclick={() => startTimer(durationMs)}>
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
    gap: 1.5rem;
    padding-block: 1.5rem 0.5rem;
    align-self: stretch;
  }
  .ti-set {
    display: flex;
    align-items: center;
    gap: 1.25rem;
  }
  .ti-digits {
    display: inline-flex;
    align-items: baseline;
  }
  /* The big digits ARE the input: type into them, the round buttons step. */
  .ti-digit {
    width: 2.1ch;
    border: none;
    background: transparent;
    font-size: var(--clock-display, 2.75rem);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--color-fg-primary);
    outline: none;
    border-radius: var(--radius-input, 6px);
  }
  .ti-digit:focus {
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
  }
  .ti-colon {
    font-size: var(--clock-display, 2.75rem);
    font-weight: 400;
    color: var(--color-fg-primary);
  }
  .ti-new :global(.ti-start) {
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
