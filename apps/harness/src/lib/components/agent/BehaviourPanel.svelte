<script lang="ts">
  /// The behaviours panel: the set the agent would act on, each on the
  /// shared ledger row grid, with the workflow/agent legend. Enabling and
  /// disabling stays in Settings; this shows live state, honestly.
  import { Activity } from "@lucide/svelte";
  import TimelineRow from "./TimelineRow.svelte";
  import AlertBanner from "./AlertBanner.svelte";
  import type { BehaviourReport, Tone } from "$lib/ledger";

  let { report }: { report: BehaviourReport | null } = $props();

  /// The detail line: description plus, when disabled, the reason.
  function detailOf(b: BehaviourReport["behaviours"][number]): { text: string; tone?: Tone }[] {
    const parts: { text: string; tone?: Tone }[] = [];
    if (b.description) parts.push({ text: b.description });
    if (!b.enabled && b.disabledReason) parts.push({ text: b.disabledReason, tone: "warn" });
    return parts;
  }
</script>

{#if !report}
  <p class="empty">Behaviour status unavailable.</p>
{:else if report.behaviours.length === 0 && report.errors.length === 0}
  <p class="empty">No agent behaviours are installed.</p>
{:else}
  <div class="head">
    <p class="hint">
      <Activity size={14} strokeWidth={1.75} />
      The set the agent would act on. Enabling and disabling stays in Settings → AI.
    </p>
    <p class="legend">
      <span><span class="kind">workflow</span> runs deterministically with no LLM call.</span>
      <span><span class="kind">agent</span> runs a bounded LLM loop.</span>
    </p>
  </div>
  {#if report.errors.length > 0}
    <AlertBanner>
      {report.errors.length} behaviour director{report.errors.length === 1 ? "y" : "ies"} failed
      to load:
      <ul class="errors">
        {#each report.errors as err (err)}<li>{err}</li>{/each}
      </ul>
    </AlertBanner>
  {/if}
  <ul class="list">
    {#each report.behaviours as b (b.name)}
      <TimelineRow
        label={b.enabled ? "enabled" : "disabled"}
        tone={b.enabled ? "ok" : "neutral"}
        subject={b.name}
        subjectMeta={[{ text: b.kind }, { text: b.provenance }]}
        detail={detailOf(b)}
      />
    {/each}
  </ul>
{/if}

<style>
  .head {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem var(--space-row, 0.75rem) 0.625rem;
  }
  .hint {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .hint :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 1rem;
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .kind {
    font-family: var(--font-mono, monospace);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .errors {
    margin: 0.25rem 0 0;
    padding-left: 1rem;
    font-size: 0.75rem;
  }
  .empty {
    margin: 0;
    padding: 0.75rem var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  /* Row dividers live on the list, since sibling rows are separate component
     instances the row's own scoped CSS cannot pair. */
  .list :global(li + li) {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
</style>
