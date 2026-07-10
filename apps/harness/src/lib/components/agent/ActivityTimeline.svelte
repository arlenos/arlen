<script lang="ts">
  /// The activity feed: a toolbar header (entry count, the type / outcome /
  /// time filters, refresh) over the ledger rows, with the honest states:
  /// record unreadable, empty, nothing matching, tamper warning, interrupted
  /// live updates. Rendering only; the page owns the reads and the filter
  /// state, this component owns the row mapping into user language.
  import { t } from "$lib/i18n/messages";
  import { RefreshCw, ShieldAlert } from "@lucide/svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import PopoverSelect from "@arlen/ui-kit/components/ui/popover-select/popover-select.svelte";
  import TimelineRow from "./TimelineRow.svelte";
  import AlertBanner from "./AlertBanner.svelte";
  import { relativeTime } from "$lib/time";
  import {
    categorize,
    entrySentence,
    failureMarker,
    undoable,
    FILTER_CATEGORIES,
  } from "$lib/display";
  import type { ActivityEntry, ActivityPage } from "$lib/ledger";

  let {
    activity,
    entries,
    error,
    loading,
    liveStale,
    category = $bindable("all"),
    outcome = $bindable("all"),
    timeWindow = $bindable("all"),
    onrefresh,
    onmore,
    onundo,
  }: {
    /// The loaded page, `null` while unloaded or after a failed load.
    activity: ActivityPage | null;
    /// The entries after the page's filters.
    entries: ActivityEntry[];
    /// The last manual-load error, rendered honestly.
    error: string | null;
    loading: boolean;
    /// True when a background poll failed, so the data shown may be stale.
    liveStale: boolean;
    category: string;
    outcome: string;
    timeWindow: string;
    onrefresh: () => void;
    /// Load a larger window of the record.
    onmore: () => void;
    /// Undo one change; resolves false when it failed.
    onundo: (entry: ActivityEntry) => Promise<boolean>;
  } = $props();

  // Explicit filter labels; "Internet" and "Blocked" do not pluralize.
  const CATEGORY_LABELS = $derived<Record<string, string>>({
    change: $t("h.filter.cat.changes"),
    lookup: $t("h.filter.cat.lookups"),
    question: $t("h.filter.cat.questions"),
    internet: $t("h.filter.cat.internet"),
    blocked: $t("h.filter.cat.blocked"),
  });
  const CATEGORY_OPTIONS = $derived([
    { value: "all", label: $t("h.filter.type.all") },
    ...FILTER_CATEGORIES.map((c) => ({ value: c.key, label: CATEGORY_LABELS[c.key] ?? c.label })),
  ]);
  const OUTCOME_OPTIONS = $derived([
    { value: "all", label: $t("h.filter.outcome.all") },
    { value: "ok", label: $t("h.filter.outcome.ok") },
    { value: "denied", label: $t("h.filter.outcome.denied") },
    { value: "error", label: $t("h.filter.outcome.error") },
  ]);
  const TIME_OPTIONS = $derived([
    { value: "all", label: $t("h.filter.time.all") },
    { value: "1h", label: $t("h.filter.time.1h") },
    { value: "24h", label: $t("h.filter.time.24h") },
    { value: "7d", label: $t("h.filter.time.7d") },
  ]);

  const filtered = $derived(
    category !== "all" || outcome !== "all" || timeWindow !== "all",
  );

  /// The technical record behind the chevron: everything the surface line
  /// dropped, unabridged, with neutral keys.
  function detailsOf(e: ActivityEntry): { key: string; value: string }[] {
    const d: { key: string; value: string }[] = [
      { key: "Recorded as", value: e.subject || e.kind },
      { key: "Component", value: e.actor },
    ];
    if (e.relations.length > 0) d.push({ key: "Graph link", value: e.relations.join(", ") });
    if (e.resultCount !== null)
      d.push({ key: "Items", value: `${e.resultCount}` });
    if (e.durationMs !== null) d.push({ key: "Duration", value: `${e.durationMs} ms` });
    if (e.outcome) d.push({ key: "Outcome", value: e.outcome });
    d.push({ key: "Reference", value: e.entryRef });
    return d;
  }

  function markerOf(e: ActivityEntry): { text: string; tone: "warn" }[] {
    const m = failureMarker(e);
    return m ? [{ text: m, tone: "warn" }] : [];
  }
</script>

<div class="head">
  <p class="count">
    {#if !activity?.available}
      {""}
    {:else if filtered}
      {entries.length} of {activity.entries.length} shown
    {:else if activity.total > activity.entries.length}
      Latest {activity.entries.length} of {activity.total}
    {:else}
      {activity.total} {activity.total === 1 ? "entry" : "entries"}
    {/if}
    {#if liveStale}
      <span
        class="stale"
        title={$t("h.filter.stale")}
      >
        <ShieldAlert size={12} strokeWidth={2} />out of date
      </span>
    {/if}
  </p>
  <div class="controls">
    <PopoverSelect
      value={category}
      options={CATEGORY_OPTIONS}
      width="8.5rem"
      ariaLabel={$t("h.filter.byType")}
      onchange={(v) => (category = v)}
    />
    <PopoverSelect
      value={outcome}
      options={OUTCOME_OPTIONS}
      width="8.5rem"
      ariaLabel={$t("h.filter.byOutcome")}
      onchange={(v) => (outcome = v)}
    />
    <PopoverSelect
      value={timeWindow}
      options={TIME_OPTIONS}
      width="8.5rem"
      ariaLabel={$t("h.filter.byTime")}
      onchange={(v) => (timeWindow = v)}
    />
    <Button variant="ghost" size="sm" disabled={loading} onclick={onrefresh}>
      <RefreshCw size={14} class={loading ? "spin" : ""} />
      Refresh
    </Button>
  </div>
</div>

{#if activity?.tampered}
  <AlertBanner>
    This record failed a safety check. Entries may have been changed outside the app.
  </AlertBanner>
{/if}

{#if error || (activity && !activity.available)}
  <p class="empty">Can't read the activity record right now.</p>
{:else if !activity}
  <p class="empty">Loading activity</p>
{:else if activity.entries.length === 0}
  <p class="empty">No activity yet. When the AI does something for you, it appears here.</p>
{:else if entries.length === 0}
  <p class="empty">Nothing matches these filters.</p>
{:else}
  <ul class="list">
    {#each entries as entry (entry.entryRef)}
      {@const cat = categorize(entry.kind)}
      <TimelineRow
        label={cat.label}
        tone={cat.tone}
        subject={entrySentence(entry)}
        subjectMeta={markerOf(entry)}
        details={detailsOf(entry)}
        time={relativeTime(entry.timestampMicros)}
        undoable={undoable(entry) && !activity.tampered}
        onundo={() => onundo(entry)}
      />
    {/each}
  </ul>
  {#if !filtered && activity.total > activity.entries.length}
    <div class="more">
      <Button variant="ghost" size="sm" disabled={loading} onclick={onmore}>
        Show older entries
      </Button>
    </div>
  {/if}
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.5rem var(--space-row, 0.75rem);
  }
  .count {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
    min-width: 0;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    white-space: nowrap;
  }
  .stale {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--color-error);
    font-size: 0.75rem;
  }
  .stale :global(svg) {
    flex-shrink: 0;
  }
  .controls {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
    flex-shrink: 0;
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
  .more {
    display: flex;
    justify-content: center;
    padding: 0.25rem var(--space-row, 0.75rem) 0.5rem;
  }
</style>
