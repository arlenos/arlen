<script lang="ts">
  /// The middle column: search over the open folder, then one row per message -
  /// unread dot, sender, subject, a one-line snippet, the time written short.
  /// The selected row speaks the sidebar's selection language so the two rails
  /// read as one system.
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { t, locale } from "$lib/i18n/messages";
  import type { Envelope } from "$lib/stores/mailbox";

  let {
    rows,
    selectedId,
    onselect,
  }: {
    rows: Envelope[];
    selectedId: string | null;
    onselect: (id: string) => void;
  } = $props();

  let query = $state("");

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter(
      (e) =>
        e.from.toLowerCase().includes(q) ||
        e.subject.toLowerCase().includes(q) ||
        e.snippet.toLowerCase().includes(q),
    );
  });

  /// Today shows the clock time, everything older a short date - the reading
  /// pane carries the full sent line, the list only orients.
  function listDate(ms: number, loc: string): string {
    const d = new Date(ms);
    const sameDay = new Date().toDateString() === d.toDateString();
    return sameDay
      ? new Intl.DateTimeFormat(loc, { timeStyle: "short" }).format(d)
      : new Intl.DateTimeFormat(loc, { day: "numeric", month: "short" }).format(d);
  }
</script>

<div class="list">
  <div class="list-search">
    <SearchField id="mail-search" bind:value={query} placeholder={$t("ml.search")} aria-label={$t("ml.search")} />
  </div>
  <div class="rows">
    {#each filtered as e (e.id)}
      <button type="button" class="row" class:on={selectedId === e.id} id={`msg-${e.id}`} onclick={() => onselect(e.id)}>
        <span class="dot" class:unread={e.unread} aria-hidden="true"></span>
        <span class="row-body">
          <span class="row-top">
            <span class="from" class:strong={e.unread}>{e.from}</span>
            <span class="when">{listDate(e.dateMs, $locale)}</span>
          </span>
          <span class="subject" class:strong={e.unread}>{e.subject}</span>
          <span class="snippet">{e.snippet}</span>
        </span>
      </button>
    {:else}
      <p class="empty">{rows.length === 0 ? $t("ml.emptyFolder") : $t("ml.noMatch")}</p>
    {/each}
  </div>
</div>

<style>
  /* The column width and its border live on the page's list column, so the
     opened-file strip and the example banner share the same edge. */
  .list {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .list-search {
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid var(--color-border-default, #2a2a2a);
  }
  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.35rem;
  }
  .row {
    display: flex;
    gap: 0.5rem;
    width: 100%;
    align-items: flex-start;
    padding: 0.5rem 0.55rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .row.on {
    background: color-mix(in srgb, var(--color-fg-primary) 9%, transparent);
  }
  .row:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }
  .dot {
    flex-shrink: 0;
    width: 0.4rem;
    height: 0.4rem;
    margin-top: 0.45rem;
    border-radius: var(--radius-full, 9999px);
  }
  .dot.unread {
    background: var(--color-accent, #7aa2f7);
  }
  .row-body {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
    flex: 1;
  }
  .row-top {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }
  .from {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm, 13px);
    color: var(--color-fg-primary);
  }
  .when {
    flex-shrink: 0;
    font-size: var(--text-2xs, 11px);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm, 13px);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
  }
  .strong {
    font-weight: 600;
  }
  .snippet {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .empty {
    margin: 1rem 0.6rem;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
