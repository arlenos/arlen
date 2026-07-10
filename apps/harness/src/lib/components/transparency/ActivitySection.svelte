<script lang="ts">
  /// Activity: a compact slice of what the background agent did, newest
  /// first, with per-item undo. The full filterable record lives on the
  /// Activity surface (/agent); this links there rather than duplicating
  /// it. Reuses the built TimelineRow so the rows read exactly like the
  /// full timeline. Rendering only; the page owns the reads and the undo.
  import { t } from "$lib/i18n/messages";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { ArrowRight } from "@lucide/svelte";
  import TimelineRow from "$lib/components/agent/TimelineRow.svelte";
  import { categorize, entrySentence, failureMarker, undoable } from "$lib/display";
  import { relativeTime } from "$lib/time";
  import type { ActivityEntry, ActivityPage } from "$lib/ledger";
  import SectionState from "./SectionState.svelte";

  let {
    activity,
    entries,
    loaded,
    onundo,
    onseeall,
  }: {
    /// The loaded page, for the honest readable/empty states.
    activity: ActivityPage | null;
    /// The recent agent-action slice the page selected.
    entries: ActivityEntry[];
    loaded: boolean;
    onundo: (entry: ActivityEntry) => Promise<boolean>;
    onseeall: () => void;
  } = $props();

  function markerOf(e: ActivityEntry): { text: string; tone: "warn" }[] {
    const m = failureMarker(e);
    return m ? [{ text: m, tone: "warn" }] : [];
  }
</script>

{#if !loaded}
  <SectionState message={$t("h.activity.loading")} />
{:else if activity === null || !activity.available}
  <SectionState message={$t("h.activity.cantRead")} />
{:else if entries.length === 0}
  <SectionState message={$t("h.activity.none")} />
{:else}
  <ul class="list">
    {#each entries as entry (entry.entryRef)}
      {@const cat = categorize(entry.kind)}
      <TimelineRow
        label={cat.label}
        tone={cat.tone}
        subject={entrySentence(entry)}
        subjectMeta={markerOf(entry)}
        time={relativeTime(entry.timestampMicros)}
        undoable={undoable(entry) && !activity.tampered}
        onundo={() => onundo(entry)}
      />
    {/each}
  </ul>
  <div class="more">
    <Button id="transparency-activity-seeall" variant="ghost" size="sm" onclick={onseeall}>
      See everything it did
      <ArrowRight size={14} strokeWidth={2} />
    </Button>
  </div>
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
    display: flex;
    justify-content: center;
    padding: 0.25rem var(--space-row, 0.75rem) 0.5rem;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
</style>
