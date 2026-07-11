<script lang="ts">
  /// Reads: what the AI has actually read (the anti-Recall payoff). The
  /// audit-reader feed (`ai_reads_recent`) now exists, so this lists the
  /// recent reads, newest first, the same way the Activity section lists
  /// actions - reuse `TimelineRow` so the rows read identically. Reads are
  /// not actions, so there is no undo. Honest states throughout: never a
  /// false "nothing read" - an unreadable feed says so. Rendering only; the
  /// page owns the read.
  import { t } from "$lib/i18n/messages";
  import TimelineRow from "$lib/components/agent/TimelineRow.svelte";
  import { categorize, entrySentence } from "$lib/display";
  import { relativeTime } from "$lib/time";
  import type { ActivityEntry, ActivityPage } from "$lib/ledger";
  import type { Capability } from "$lib/capability";
  import SectionState from "./SectionState.svelte";

  let {
    reads,
    entries,
    loaded,
    capability,
  }: {
    /// The loaded reads page, for the honest readable/empty states.
    reads: ActivityPage | null;
    /// The recent-reads slice the page selected.
    entries: ActivityEntry[];
    loaded: boolean;
    capability: Capability | null;
  } = $props();

  const off = $derived(capability !== null && !capability.enabled);
  // How many more reads the ledger holds beyond the slice shown.
  const more = $derived(reads ? Math.max(0, reads.total - entries.length) : 0);
</script>

{#if off}
  <SectionState tag="AI is off" tone="off" message={$t("h.reads.off")} />
{:else if !loaded}
  <SectionState message={$t("h.reads.loading")} />
{:else if reads === null || !reads.available}
  <SectionState
    tag={$t("h.reads.cantTitle")}
    tone="info"
    message={$t("h.reads.cantRead")}
  />
{:else if entries.length === 0}
  <SectionState message={$t("h.reads.none")} />
{:else}
  <ul class="list">
    {#each entries as entry (entry.entryRef)}
      {@const cat = categorize(entry.kind)}
      {@const catLabel = cat.labelKey ? $t(cat.labelKey) : cat.key}
      <TimelineRow
        label={catLabel}
        tone={cat.tone}
        subject={entrySentence(entry, $t)}
        time={relativeTime(entry.timestampMicros)}
      />
    {/each}
  </ul>
  {#if more > 0}
    <p class="more">{more} more {more === 1 ? "read" : "reads"} recorded.</p>
  {/if}
{/if}

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .list :global(li + li) {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .more {
    margin: 0;
    padding: 0.5rem var(--space-row, 0.75rem);
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    text-align: center;
  }
</style>
