<script lang="ts">
  /// Focus (clock-app.md, Tim's locked decision): a timer with ENFORCEMENT,
  /// not a themed stopwatch. The card states exactly what the session holds
  /// back - from the daemon's own list, never asserted - and ending early is
  /// always one click. No streaks, no music, no dashboard.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { clock, tick, startFocus, endFocus, setFocusConfig } from "$lib/stores/clock";
  import { fmtDuration } from "$lib/format";
  import { t } from "$lib/i18n/messages";

  const heldText = (held: string[]) =>
    held.length === 0 ? $t("c.fo.heldNothing") : $t("c.fo.held", { what: held.map((h) => (h === "notifications" ? $t("c.fo.notifications") : h)).join(", ") });
</script>

<div class="fo">
  {#if $clock}
    {#if $clock.focus}
      {@const f = $clock.focus}
      <div class="fo-card">
        <span class="fo-phase">{f.phase === "focus" ? $t("c.fo.phase.focus") : $t("c.fo.phase.break")}</span>
        <span class="fo-remaining">{fmtDuration(f.ends_at - $tick)}</span>
        <span class="fo-round">{$t("c.fo.round", { n: f.round, total: f.rounds })}</span>
        <div class="fo-dots" aria-hidden="true">
          {#each Array(f.rounds) as _, i (i)}
            <span class="fo-dot" class:done={i < f.round}></span>
          {/each}
        </div>
        <p class="fo-held">{heldText(f.held)}</p>
        <Button variant="outline" id="end-focus" onclick={endFocus}>{$t("c.fo.end")}</Button>
      </div>
    {:else}
      <div class="fo-card idle">
        <p class="fo-idle">{$t("c.fo.idle")}</p>
        <Button id="start-focus" onclick={startFocus}>{$t("c.fo.start")}</Button>
      </div>
      {@const cfg = $clock.focus_config}
      <Section>
        <Row id="focus-len" label={$t("c.fo.focusLen")}>
          {#snippet control()}
            <NumberInput value={cfg.focus_min} min={5} max={120} unit={$t("c.fo.min")} ariaLabel={$t("c.fo.focusLen")} onchange={(v) => setFocusConfig({ ...cfg, focus_min: v })} />
          {/snippet}
        </Row>
        <Row id="break-len" label={$t("c.fo.breakLen")}>
          {#snippet control()}
            <NumberInput value={cfg.break_min} min={1} max={60} unit={$t("c.fo.min")} ariaLabel={$t("c.fo.breakLen")} onchange={(v) => setFocusConfig({ ...cfg, break_min: v })} />
          {/snippet}
        </Row>
        <Row id="focus-rounds" label={$t("c.fo.rounds")}>
          {#snippet control()}
            <NumberInput value={cfg.rounds} min={1} max={12} ariaLabel={$t("c.fo.rounds")} onchange={(v) => setFocusConfig({ ...cfg, rounds: v })} />
          {/snippet}
        </Row>
      </Section>
    {/if}
  {/if}
</div>

<style>
  .fo {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-width: 34rem;
    padding: 0.9rem 1rem 1.5rem;
  }
  .fo-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    padding: 1.5rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .fo-phase {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .fo-remaining {
    font-size: 2.6rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    color: var(--color-fg-primary);
  }
  .fo-round {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .fo-dots {
    display: flex;
    gap: 0.35rem;
    padding: 0.15rem 0;
  }
  .fo-dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--color-fg-primary) 18%, transparent);
  }
  .fo-dot.done {
    background: var(--color-fg-primary);
  }
  /* The enforcement honesty: what is held, from the daemon, stated plainly. */
  .fo-held {
    margin: 0.25rem 0 0.5rem;
    max-width: 24rem;
    font-size: var(--text-xs);
    line-height: 1.5;
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .fo-card.idle {
    gap: 0.9rem;
    padding: 2rem 1rem;
  }
  .fo-idle {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
</style>
