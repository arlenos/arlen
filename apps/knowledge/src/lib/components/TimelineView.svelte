<script lang="ts">
  /// The Timeline spine (KA-R2): day-grouped typed events with reconstructed
  /// session blocks, the scrub rail above, and the trust controls in the head
  /// (decision 5: the timeline is on by default ONLY because pause, export and
  /// delete are visible first-class controls, with an honest statement of what
  /// is recorded). Event rows reuse the privacy page's sentence anatomy: quiet
  /// verb, emphasized object - recall of the user's data, not an app log.
  import { onMount } from "svelte";
  import {
    ChevronRight,
    Pause,
    Play,
    FileText,
    Pencil,
    SquareTerminal,
    Crosshair,
    Sparkles,
    ArrowDownToLine,
  } from "lucide-svelte";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    days,
    timelineMocked,
    timelineUnavailable,
    paused,
    pauseUnavailable,
    pendingMenuAction,
    loadTimeline,
    setPaused,
    exportTimeline,
    deleteRange,
    dayLabel,
    clock,
    type TimelineEvent,
    type TimelineKind,
  } from "$lib/stores/timeline";
  import TimelineScrub from "./TimelineScrub.svelte";
  import { t, locale } from "$lib/i18n/messages";

  let { onselect }: { onselect: (event: TimelineEvent) => void } = $props();

  onMount(loadTimeline);

  const KIND_ICONS: Record<TimelineKind, typeof FileText> = {
    opened: FileText,
    edited: Pencil,
    ran: SquareTerminal,
    focus: Crosshair,
    agent: Sparkles,
    imported: ArrowDownToLine,
  };

  // Session blocks fold; the day's lone events always show.
  let openSessions = $state<Set<string>>(new Set());
  function toggleSession(id: string): void {
    const next = new Set(openSessions);
    next.has(id) ? next.delete(id) : next.add(id);
    openSessions = next;
  }

  // The scrub follows the scroll and drives it: jumping scrolls the day anchor
  // into view; scrolling picks the day nearest the top as active.
  let scroller = $state<HTMLDivElement | null>(null);
  let activeIndex = $state(0);
  let jumping = false;
  let jumpTimer: ReturnType<typeof setTimeout> | null = null;

  function jumpTo(index: number): void {
    const list = $days;
    if (!list || !scroller) return;
    activeIndex = index;
    const el = scroller.querySelector(`[data-day="${list[index].date}"]`);
    if (el) {
      // One shared settle window: rapid jumps must not let an earlier timeout
      // hand control back to the scroll listener mid-sequence.
      jumping = true;
      if (jumpTimer) clearTimeout(jumpTimer);
      el.scrollIntoView({ block: "start" });
      jumpTimer = setTimeout(() => (jumping = false), 150);
    }
  }

  function onScroll(): void {
    const list = $days;
    if (!list || !scroller || jumping) return;
    const top = scroller.getBoundingClientRect().top;
    let best = 0;
    let bestDist = Infinity;
    for (let i = 0; i < list.length; i++) {
      const el = scroller.querySelector(`[data-day="${list[i].date}"]`);
      if (!el) continue;
      const dist = Math.abs(el.getBoundingClientRect().top - top);
      if (dist < bestDist) {
        bestDist = dist;
        best = i;
      }
    }
    activeIndex = best;
  }

  // The what's-recorded disclosure carries the honest statement; the export
  // and delete ACTIONS live in the shell's app menu (lib/menu.ts), and this
  // surface only resolves what a menu action started: run the export, or ask
  // the destructive confirm.
  let disclosureOpen = $state(false);
  let exportFailed = $state(false);
  let deleteFailed = $state(false);
  /// Where the last export landed, so the line can name the file.
  let exportedTo = $state<string | null>(null);

  let pendingDelete = $state<{ from: number; label: string } | null>(null);
  function midnightToday(): number {
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    return Math.floor(d.getTime() / 1000);
  }
  async function confirmDelete(): Promise<void> {
    if (pendingDelete === null) return;
    deleteFailed = !(await deleteRange(pendingDelete.from));
    pendingDelete = null;
  }

  $effect(() => {
    const action = $pendingMenuAction;
    if (!action) return;
    pendingMenuAction.set(null);
    if (action === "export") {
      void exportTimeline().then((path) => {
        exportFailed = path === null;
        exportedTo = path;
      });
    } else if (action === "deleteToday") {
      pendingDelete = { from: midnightToday(), label: $t("k.tl.rangeToday") };
    } else {
      pendingDelete = { from: 0, label: $t("k.tl.rangeAll") };
    }
  });

  const empty = $derived($days !== null && $days.length === 0);
</script>

<div class="tl">
  <header class="tl-head">
    <div class="tl-head-line">
      {#if $timelineMocked}
        <span class="tl-sample">{$t("k.sample")}</span>
      {/if}
      <span class="tl-spacer"></span>
      <button type="button" class="tl-pause" class:on={$paused} onclick={() => setPaused(!$paused)}>
        {#if $paused}
          <Play size={13} strokeWidth={2} />
          {$t("k.tl.resume")}
        {:else}
          <Pause size={13} strokeWidth={2} />
          {$t("k.tl.pause")}
        {/if}
      </button>
      <button type="button" class="tl-whats" class:open={disclosureOpen} onclick={() => (disclosureOpen = !disclosureOpen)}>
        <ChevronRight size={13} strokeWidth={2} />
        {$t("k.tl.whats")}
      </button>
    </div>
    {#if $pauseUnavailable}
      <p class="tl-paused-line" role="alert">{$t("k.tl.pauseUnavailable")}</p>
    {/if}
    {#if $paused}
      <p class="tl-paused-line">{$t("k.tl.pausedLine")}</p>
    {/if}
    {#if disclosureOpen}
      <div class="tl-disclosure">
        <p class="tl-statement">{$t("k.tl.statement")}</p>
        <p class="tl-statement-menu">{$t("k.tl.menuHint")}</p>
        {#if deleteFailed}
          <p class="tl-fail">{$t("k.tl.deleteFail")}</p>
        {/if}
        {#if exportFailed}
          <p class="tl-fail">{$t("k.tl.exportFail")}</p>
        {:else if exportedTo}
          <!-- The path, not just "done": the file is the point of the export. -->
          <p class="tl-statement">{$t("k.tl.exportedTo", { path: exportedTo })}</p>
        {/if}
      </div>
    {/if}
  </header>

  {#if $days && $days.length > 1}
    <TimelineScrub days={$days} {activeIndex} onjump={jumpTo} />
  {/if}

  <div class="tl-scroll" bind:this={scroller} onscroll={onScroll}>
    {#if empty}
      <!-- "nothing recorded" and "could not read" are the same empty spine. -->
      <p class="tl-empty">{$timelineUnavailable ? $t("k.tl.unavailable") : $t("k.empty.timeline")}</p>
    {:else if $days}
      {#each $days as day, i (day.date)}
        <section class="tl-day" data-day={day.date}>
          <h2 class="tl-day-head">{dayLabel(day.date, $locale)}</h2>
          {#each day.items as item (item.kind === "event" ? item.event.id : item.session.id)}
            {#if item.kind === "event"}
              {@const Icon = KIND_ICONS[item.event.kind]}
              <button type="button" class="tl-row" onclick={() => onselect(item.event)}>
                <span class="tl-icon"><Icon size={14} strokeWidth={1.75} /></span>
                <span class="tl-verb">{$t(item.event.verb)}</span>
                <span class="tl-object">
                  {item.event.object}
                  {#if item.event.project}<span class="tl-chip">{item.event.project}</span>{/if}
                </span>
                <span class="tl-source">{item.event.source}</span>
                <span class="tl-time">{clock(item.event.at, $locale)}</span>
              </button>
            {:else}
              {@const s = item.session}
              {@const open = openSessions.has(s.id)}
              <div class="tl-session" class:open>
                <button type="button" class="tl-session-head" onclick={() => toggleSession(s.id)}>
                  <span class="tl-session-chev" class:open><ChevronRight size={13} strokeWidth={2} /></span>
                  <span class="tl-session-title">
                    <span class="tl-session-kind">{$t("k.tl.session")}</span>
                    {s.title}
                  </span>
                  <span class="tl-session-meta">
                    {$t("k.tl.sessionMeta", { n: s.events.length })}
                  </span>
                  <span class="tl-time">{$t("k.tl.span", { from: clock(s.from, $locale), to: clock(s.to, $locale) })}</span>
                </button>
                {#if open}
                  <div class="tl-session-body">
                    {#each s.events as e (e.id)}
                      {@const Icon = KIND_ICONS[e.kind]}
                      <button type="button" class="tl-row nested" onclick={() => onselect(e)}>
                        <span class="tl-icon"><Icon size={14} strokeWidth={1.75} /></span>
                        <span class="tl-verb">{$t(e.verb)}</span>
                        <span class="tl-object">{e.object}</span>
                        <span class="tl-source">{e.source}</span>
                        <span class="tl-time">{clock(e.at, $locale)}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
          {/each}
        </section>
      {/each}
    {/if}
  </div>
</div>

<ConfirmDialog
  open={pendingDelete !== null}
  title={$t("k.tl.deleteTitle")}
  message={$t("k.tl.deleteMsg", { range: pendingDelete?.label ?? "" })}
  confirmLabel={$t("k.tl.deleteConfirm")}
  variant="destructive"
  onConfirm={confirmDelete}
  onCancel={() => (pendingDelete = null)}
/>

<style>
  .tl {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .tl-head {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.6rem 1.1rem 0.2rem;
  }
  .tl-head-line {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .tl-sample {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .tl-spacer {
    flex: 1;
  }
  /* Pause is a real toggle: pressed look while recording is off. */
  .tl-pause,
  .tl-whats {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.55rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    cursor: pointer;
  }
  .tl-pause:hover,
  .tl-whats:hover {
    color: var(--color-fg-primary);
  }
  .tl-pause.on {
    background: color-mix(in srgb, var(--color-warning, #ca8a04) 12%, transparent);
    border-color: color-mix(in srgb, var(--color-warning, #ca8a04) 35%, transparent);
    color: var(--color-fg-primary);
  }
  .tl-whats :global(svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .tl-whats.open :global(svg) {
    transform: rotate(90deg);
  }
  /* Status line only in the deviating state (the rule): recording paused. */
  .tl-paused-line {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-warning, #ca8a04) 90%, var(--color-fg-primary));
  }

  .tl-disclosure {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
  }
  .tl-statement {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .tl-statement-menu {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .tl-fail {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }

  .tl-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.4rem 1.1rem 1.25rem;
  }
  .tl-empty {
    margin: 1rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .tl-day {
    display: flex;
    flex-direction: column;
    scroll-margin-top: 0.25rem;
  }
  .tl-day-head {
    margin: 0.9rem 0 0.25rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }

  /* The sentence row: quiet verb, emphasized object, quiet source, tabular
     time. One grid so the columns align down the whole spine. */
  /* The time column is fixed so the flexible column resolves identically on
     every row; the sources then share one clean right seam. */
  .tl-row {
    display: grid;
    grid-template-columns: max-content max-content minmax(0, 1fr) max-content 5.5rem;
    align-items: baseline;
    column-gap: 0.625rem;
    width: 100%;
    padding: 0.3rem 0.375rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .tl-row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .tl-icon {
    display: inline-flex;
    align-self: center;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  /* Fixed verb column so the objects form one scannable column down the whole
     spine (each row is its own grid; a shared width is what aligns them). */
  .tl-verb {
    width: 4.5rem;
    text-align: end;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .tl-object {
    display: inline-flex;
    align-items: baseline;
    gap: 0.4rem;
    min-width: 0;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tl-chip {
    flex-shrink: 0;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-2xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
  }
  .tl-source {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
    white-space: nowrap;
  }
  .tl-time {
    justify-self: end;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    white-space: nowrap;
  }

  /* A reconstructed session: one foldable block in the spine, its events
     indented under it. */
  .tl-session {
    display: flex;
    flex-direction: column;
    margin: 0.15rem 0;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card);
  }
  .tl-session-head {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) max-content max-content;
    align-items: baseline;
    column-gap: 0.625rem;
    padding: 0.45rem 0.625rem;
    border: none;
    background: transparent;
    text-align: start;
    cursor: pointer;
    border-radius: inherit;
  }
  .tl-session-head:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .tl-session-chev {
    display: inline-flex;
    align-self: center;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .tl-session-chev.open {
    transform: rotate(90deg);
  }
  .tl-session-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tl-session-kind {
    margin-inline-end: 0.375rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .tl-session-meta {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    white-space: nowrap;
  }
  .tl-session-body {
    display: flex;
    flex-direction: column;
    padding: 0 0.375rem 0.375rem 1.4rem;
  }
</style>
