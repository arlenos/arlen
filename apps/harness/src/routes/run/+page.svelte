<script lang="ts">
  /// The run view (ai-transparency-surface.md §4b): a run while it runs, not
  /// the record of one. Four parts, in reading order: where it is against its
  /// bound, then each step with the tokens it burned as a bar, how long it
  /// took or has been waiting, and its reasoning folded beneath it.
  ///
  /// Everything comes from the daemon's ledger, never from what the model
  /// says about itself (§H integrity rule): steps are the entries of one
  /// `call_chain_id`, a step without a duration is in flight (§4c). Tokens
  /// per step, the reasoning trace and the bound have no feed yet; each says
  /// so where it would be, instead of drawing a zero. Without a host the page
  /// shows a sample run with every part filled, marked as a sample.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t, locale } from "$lib/i18n/messages";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { Progress } from "@arlen/ui-kit/components/ui/progress";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
    CollapsibleChevron,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { tauriAvailable } from "$lib/tauri";
  import { entrySentence } from "$lib/display";
  import { groupRuns, running, sampleRun, type Run } from "$lib/run";
  import type { ActivityPage } from "$lib/ledger";

  let run = $state<Run | null>(null);
  let unreadable = $state(false);
  let loaded = $state(false);
  let nowMicros = $state(Date.now() * 1000);
  const sample = !tauriAvailable;

  async function load(): Promise<void> {
    if (sample) {
      run = sampleRun(Date.now() * 1000);
      loaded = true;
      return;
    }
    try {
      const page = await invoke<ActivityPage>("ai_activity_recent", { limit: 200 });
      unreadable = !page.available;
      run = groupRuns(page.entries)[0] ?? null;
    } catch {
      unreadable = true;
      run = null;
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    void load();
    // The waiting time is the only number that moves by itself, so a second's
    // tick is all the page needs; the record is re-read every two.
    const tick = setInterval(() => (nowMicros = Date.now() * 1000), 1000);
    const poll = setInterval(() => {
      if (!sample && !document.hidden) void load();
    }, 2000);
    return () => {
      clearInterval(tick);
      clearInterval(poll);
    };
  });

  const steps = $derived(run?.steps ?? []);
  const live = $derived(run !== null && running(run));
  const maxTokens = $derived(Math.max(1, ...steps.map((s) => s.tokens ?? 0)));
  const tokensMeasured = $derived(steps.some((s) => s.tokens !== null));
  const traceMeasured = $derived(steps.some((s) => s.trace !== null));

  function seconds(ms: number): string {
    return new Intl.NumberFormat($locale, { maximumFractionDigits: 1, minimumFractionDigits: ms < 10_000 ? 1 : 0 }).format(ms / 1000);
  }
  function count(n: number): string {
    return new Intl.NumberFormat($locale).format(n);
  }
</script>

<Page title={$t("h.run.title")} description={$t("h.run.sub")}>
  <SectionGrid>
    {#if sample}
      <div class="span-full"><Notice text={$t("h.run.sample")} /></div>
    {/if}
    {#if unreadable}
      <div class="span-full"><Notice tone="error" text={$t("h.run.unreadable")} /></div>
    {/if}

    {#if loaded && !run && !unreadable}
      <Section class="span-full">
        <p class="empty">{$t("h.run.none")}</p>
      </Section>
    {:else if run}
      <Section label={$t("h.run.where")} class="span-full">
        <div class="where">
          <div class="where-line">
            <span class="state" class:live>{live ? $t("h.run.running") : $t("h.run.finished")}</span>
            <span id="run-step-of" class="step-of">
              {run.bound !== null
                ? $t("h.run.stepOf", { n: steps.length, bound: run.bound })
                : $t("h.run.step", { n: steps.length })}
            </span>
          </div>
          {#if run.bound !== null}
            <Progress value={(steps.length / run.bound) * 100} labelledby="run-step-of" />
          {:else}
            <Notice text={$t("h.run.noBound")} />
          {/if}
        </div>
      </Section>

      <Section label={$t("h.run.steps")} class="span-full">
        {#if !tokensMeasured || !traceMeasured}
          <div class="feeds">
            {#if !tokensMeasured}<Notice text={$t("h.run.tokensUnmeasured")} />{/if}
            {#if !traceMeasured}<Notice text={$t("h.run.traceUnavailable")} />{/if}
          </div>
        {/if}
        <ol class="steps">
          {#each steps as s, i (s.entry.entryRef)}
            {@const inFlight = s.entry.durationMs === null}
            <li class="step" class:in-flight={inFlight}>
              <div class="row">
                <span class="n">{i + 1}</span>
                <span class="what">{entrySentence(s.entry, $t)}</span>
                <span class="burn" aria-hidden={s.tokens === null}>
                  {#if s.tokens !== null && s.tokens > 0}
                    <span class="bar" style={`width:${(s.tokens / maxTokens) * 100}%`}></span>
                    <span class="tok">{$t("h.run.tokens", { n: count(s.tokens) })}</span>
                  {/if}
                </span>
                <span class="time">
                  {#if inFlight}
                    {$t("h.run.waiting", { s: seconds((nowMicros - s.entry.timestampMicros) / 1000) })}
                  {:else}
                    {$t("h.run.took", { s: seconds(s.entry.durationMs ?? 0) })}
                  {/if}
                </span>
              </div>
              {#if s.trace}
                <Collapsible>
                  <CollapsibleTrigger class="trace-trigger"><CollapsibleChevron size={13} />{$t("h.run.reasoning")}</CollapsibleTrigger>
                  <CollapsibleContent><p class="trace">{s.trace}</p></CollapsibleContent>
                </Collapsible>
              {/if}
            </li>
          {/each}
        </ol>
      </Section>
    {/if}
  </SectionGrid>
</Page>

<style>
  .empty {
    margin: 0;
    padding: 1rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .where {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.9rem 1rem;
  }
  .where-line {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
  }
  .state {
    font-size: var(--text-sm);
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .state.live {
    color: var(--color-accent, var(--foreground));
  }
  .step-of {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .feeds {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.75rem 1rem 0;
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0.4rem 0;
  }
  .step {
    padding: 0.45rem 1rem;
  }
  .step + .step {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  /* One grid for every row, so the bars start on one line and their lengths
     compare: a step that cost ten times its neighbour is a shape, not a number. */
  .row {
    display: grid;
    grid-template-columns: 1.5rem minmax(0, 1.4fr) minmax(8rem, 1fr) 7.5rem;
    align-items: center;
    gap: 0.75rem;
  }
  .n {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .what {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }
  .burn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  .bar {
    flex-shrink: 0;
    height: 0.5rem;
    min-width: 2px;
    max-width: calc(100% - 5.5rem);
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .tok {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .time {
    text-align: end;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .in-flight .time {
    color: var(--color-accent, var(--foreground));
    font-weight: 600;
  }
  .step :global(.trace-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin: 0.3rem 0 0 2.25rem;
    padding: 0;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .trace {
    margin: 0.35rem 0 0.2rem 2.25rem;
    max-width: 60ch;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }
</style>
