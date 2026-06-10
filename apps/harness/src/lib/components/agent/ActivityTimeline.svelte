<script lang="ts">
  /// The read-only activity timeline (trigger → gate → act → audit, newest
  /// first) with its honest states: daemon unavailable, empty ledger, no
  /// filter match, tamper warning, interrupted live updates. Rendering only;
  /// the page owns the reads and the filters.
  import { History, RefreshCw, ShieldAlert } from "@lucide/svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import TimelineRow from "./TimelineRow.svelte";
  import AlertBanner from "./AlertBanner.svelte";
  import { relativeTime } from "$lib/time";
  import { KIND_META, type ActivityEntry, type ActivityPage } from "$lib/ledger";

  let {
    activity,
    entries,
    error,
    loading,
    liveStale,
    onrefresh,
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
    onrefresh: () => void;
  } = $props();

  /// The detail line: actor, relations, result count, duration — only the
  /// parts the entry actually carries.
  function detailOf(e: ActivityEntry): { text: string }[] {
    const parts: { text: string }[] = [{ text: e.actor }];
    if (e.relations.length > 0) parts.push({ text: e.relations.join(", ") });
    if (e.resultCount !== null)
      parts.push({ text: `${e.resultCount} result${e.resultCount === 1 ? "" : "s"}` });
    if (e.durationMs !== null) parts.push({ text: `${e.durationMs} ms` });
    return parts;
  }
</script>

<div class="head">
  <p class="hint">
    <History size={14} strokeWidth={1.75} />
    <span>
      Trigger → gate → act → audit, newest first.{#if activity && activity.total > 0}
        <span class="count">{activity.total} total</span>{/if}
      {#if liveStale}
        <span
          class="stale"
          title="A background refresh failed; showing the last known data. Use Refresh."
        >
          <ShieldAlert size={12} strokeWidth={2} />live updates interrupted
        </span>
      {/if}
    </span>
  </p>
  <Button variant="ghost" size="sm" disabled={loading} onclick={onrefresh}>
    <RefreshCw size={14} class={loading ? "spin" : ""} />
    Refresh
  </Button>
</div>

{#if activity?.tampered}
  <AlertBanner>
    The audit ledger reports tampering. The entries below may be incomplete.
  </AlertBanner>
{/if}

{#if error}
  <p class="empty">Activity unavailable: {error}</p>
{:else if !activity || !activity.available}
  <p class="empty">The audit daemon is not running, so there is no activity to show yet.</p>
{:else if activity.entries.length === 0}
  <p class="empty">No AI activity recorded yet.</p>
{:else if entries.length === 0}
  <p class="empty">No activity matches the current filters.</p>
{:else}
  <ul class="list">
    {#each entries as entry (entry.entryRef)}
      {@const meta = KIND_META[entry.kind] ?? { label: entry.kind, tone: "neutral" }}
      <TimelineRow
        label={meta.label}
        tone={meta.tone}
        subject={entry.subject}
        subjectMeta={[
          {
            text: entry.outcome,
            tone: entry.outcome === "denied" || entry.outcome === "error" ? "warn" : "neutral",
          },
        ]}
        detail={detailOf(entry)}
        time={relativeTime(entry.timestampMicros)}
      />
    {/each}
  </ul>
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-row, 0.75rem);
    padding: 0.5rem var(--space-row, 0.75rem);
  }
  .hint {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0;
    min-width: 0;
    font-size: 0.8125rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .hint :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
  }
  .count {
    margin-left: 0.375rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .stale {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-left: 0.5rem;
    color: var(--color-error);
  }
  .stale :global(svg) {
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
</style>
