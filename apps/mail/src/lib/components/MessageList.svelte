<script lang="ts">
  /// The middle column: search over the open folder, then one row per message -
  /// unread dot, sender, subject, a one-line snippet, the time written short.
  /// The selected row speaks the sidebar's selection language so the two rails
  /// read as one system.
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { t, locale } from "$lib/i18n/messages";
  import type { Envelope } from "$lib/stores/mailbox";

  let {
    rows,
    selected,
    onchange,
    onopen,
    onarchive,
    ondelete,
  }: {
    rows: (Envelope & { count?: number })[];
    /// The selected message ids; one id means the reading pane shows it.
    selected: Set<string>;
    onchange: (sel: Set<string>) => void;
    /// A plain click or Enter: single-select and read.
    onopen: (id: string) => void;
    onarchive: () => void;
    ondelete: () => void;
  } = $props();

  /// The shift-range anchor: the last row a plain or ctrl click landed on.
  let anchor = $state<string | null>(null);

  let query = $state("");
  // All or unread only. Session state, deliberately not persisted: a filter a
  // person forgot about is how "my mail disappeared" reports happen.
  let show = $state("all");

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let list = show === "unread" ? rows.filter((e) => e.unread) : rows;
    if (!q) return list;
    return list.filter(
      (e) =>
        e.from.toLowerCase().includes(q) ||
        e.subject.toLowerCase().includes(q) ||
        e.snippet.toLowerCase().includes(q),
    );
  });

  function clickRow(e: MouseEvent, id: string): void {
    const order = filtered.map((x) => x.id);
    if (e.shiftKey && anchor && order.includes(anchor)) {
      const a = order.indexOf(anchor);
      const b = order.indexOf(id);
      const range = order.slice(Math.min(a, b), Math.max(a, b) + 1);
      onchange(new Set(range));
    } else if (e.ctrlKey || e.metaKey) {
      const next = new Set(selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      anchor = id;
      onchange(next);
    } else {
      anchor = id;
      onopen(id);
    }
  }

  /// List keyboard: arrows or j/k walk the visible order and read as they go,
  /// Enter reads the anchored row, Delete removes, e archives - the desktop
  /// conventions, scoped to the list so typing in search stays typing.
  function keydown(e: KeyboardEvent): void {
    if ((e.target as HTMLElement | null)?.closest("input, textarea")) return;
    const order = filtered.map((x) => x.id);
    if (order.length === 0) return;
    const cur = anchor ?? [...selected][0] ?? null;
    const idx = cur ? order.indexOf(cur) : -1;
    const step = (to: number) => {
      const id = order[Math.max(0, Math.min(order.length - 1, to))];
      anchor = id;
      onopen(id);
      document.getElementById(`msg-${id}`)?.focus();
    };
    if (e.key === "ArrowDown" || e.key === "j") {
      e.preventDefault();
      step(idx + 1);
    } else if (e.key === "ArrowUp" || e.key === "k") {
      e.preventDefault();
      step(idx - 1);
    } else if (e.key === "Enter" && cur) {
      e.preventDefault();
      onopen(cur);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      ondelete();
    } else if (e.key === "e") {
      e.preventDefault();
      onarchive();
    }
  }

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
    <SegmentedControl
      id="mail-filter"
      bind:value={show}
      options={[
        { value: "all", label: $t("ml.filter.all") },
        { value: "unread", label: $t("ml.filter.unread") },
      ]}
    />
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- Not in the tab order itself: Tab lands on the first row button, and the
       key handler hears the rows' events as they bubble. -->
  <div class="rows" role="listbox" aria-multiselectable="true" aria-label={$t("ml.search")} tabindex={-1} onkeydown={keydown}>
    {#each filtered as e (e.id)}
      <button
        type="button"
        class="row"
        class:on={selected.has(e.id)}
        id={`msg-${e.id}`}
        role="option"
        aria-selected={selected.has(e.id)}
        onclick={(ev) => clickRow(ev, e.id)}
      >
        <span class="dot" class:unread={e.unread} aria-hidden="true"></span>
        <span class="row-body">
          <span class="row-top">
            <span class="from" class:strong={e.unread}>{e.from}</span>
            <span class="when">{listDate(e.dateMs, $locale)}</span>
          </span>
          <span class="subject-line">
            <span class="subject" class:strong={e.unread}>{e.subject}</span>
            {#if e.count}
              <span class="count" aria-label={$t("ml.threadCount", { n: e.count })}>{e.count}</span>
            {/if}
          </span>
          <span class="snippet">{e.snippet}</span>
        </span>
      </button>
    {:else}
      <p class="empty">
        {rows.length === 0
          ? $t("ml.emptyFolder")
          : show === "unread" && query.trim() === ""
            ? $t("ml.noUnread")
            : $t("ml.noMatch")}
      </p>
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
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
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
  .subject-line {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
  }
  .count {
    flex-shrink: 0;
    padding: 0 0.3rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    font-size: var(--text-2xs, 11px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .subject {
    min-width: 0;
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
