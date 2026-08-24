<script lang="ts">
  /// The time grid: one column per day, an all-day strip pinned on top, timed
  /// events as positioned blocks that split the column when they overlap. Also
  /// the day view - the same engine with one column. The grid's slot height
  /// stays >= 24px per half hour (calendar-app.md §7a c) and the initial
  /// scroll lands the working morning in view.
  ///
  /// No dragging in v1: creating and moving events are the form's job, which
  /// also keeps SC 2.5.7's single-pointer requirement moot here.
  import { t, locale } from "$lib/i18n/messages";
  import { isToday } from "$lib/wording";
  import { layoutDay, parseYmd, ymd, type AgendaEvent } from "$lib/stores/calendar";
  import EventPopover from "./EventPopover.svelte";

  let {
    days,
    events,
  }: {
    /// The visible dates (YYYY-MM-DD), Monday-first for a week, one for a day.
    days: string[];
    events: AgendaEvent[];
  } = $props();

  const HOUR = 48; // px per hour -> 24px per half-hour slot, the floor.

  const byDay = $derived.by(() => {
    const map = new Map<string, AgendaEvent[]>();
    for (const d of days) map.set(d, []);
    for (const e of events) map.get(e.date)?.push(e);
    return map;
  });

  const now = new Date();
  const nowTop = $derived((now.getHours() * 60 + now.getMinutes()) * (HOUR / 60));

  function headParts(d: string): { dow: string; num: number } {
    const date = parseYmd(d);
    return {
      dow: new Intl.DateTimeFormat($locale, { weekday: "short" }).format(date),
      num: date.getDate(),
    };
  }

  let scroller = $state<HTMLElement | null>(null);
  $effect(() => {
    // Land the morning in view once the grid exists; 8:00 minus a breath.
    if (scroller) scroller.scrollTop = 7.5 * HOUR;
  });
</script>

<div class="week" style="--cols: {days.length}">
  <div class="head-row">
    <span class="gutter"></span>
    {#each days as d (d)}
      {@const p = headParts(d)}
      <span class="day-head" class:today={isToday(d, new Date())}>
        <span class="dow">{p.dow}</span>
        <span class="num">{p.num}</span>
      </span>
    {/each}
  </div>

  <div class="allday-row">
    <span class="gutter allday-label">{$t("cal.allDayRow")}</span>
    {#each days as d (d)}
      <span class="allday-cell">
        {#each (byDay.get(d) ?? []).filter((e) => e.time === null) as e (e.uid + e.date)}
          <EventPopover event={e}>
            {#snippet children(props: Record<string, unknown>)}
              <button type="button" class="allday-pill" {...props}>{e.summary}</button>
            {/snippet}
          </EventPopover>
        {/each}
      </span>
    {/each}
  </div>

  <div class="scroll" bind:this={scroller}>
    <div class="grid" style="height: {24 * HOUR}px">
      <div class="gutter hours">
        {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
          <span class="hour" style="top: {h * HOUR}px">{String(h).padStart(2, "0")}:00</span>
        {/each}
      </div>
      {#each days as d (d)}
        {@const today = isToday(d, new Date())}
        <div class="day-col" class:today>
          {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
            <span class="line" style="top: {h * HOUR}px" aria-hidden="true"></span>
          {/each}
          {#if today && ymd(now) === d}
            <span class="now" style="top: {nowTop}px" aria-hidden="true"></span>
          {/if}
          {#each layoutDay(byDay.get(d) ?? []) as b (b.event.uid + b.event.date + (b.event.time ?? ""))}
            <EventPopover event={b.event}>
              {#snippet children(props: Record<string, unknown>)}
                <button
                  type="button"
                  class="block"
                  style="top: {(b.startMin * HOUR) / 60}px; height: {Math.max(((b.endMin - b.startMin) * HOUR) / 60, 20)}px; left: calc({(b.col / b.cols) * 100}% + 2px); width: calc({100 / b.cols}% - 4px);"
                  {...props}
                >
                  {#if b.endMin - b.startMin < 40}
                    <!-- A short block holds one line, and the line that matters
                         is the name; the time is where the block sits. -->
                    <span class="b-title">{b.event.summary}</span>
                  {:else}
                    <span class="b-title">{b.event.summary}</span>
                    <span class="b-time">{b.event.time}{#if b.event.end_time}&#8211;{b.event.end_time}{/if}</span>
                  {/if}
                </button>
              {/snippet}
            </EventPopover>
          {/each}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .week {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .head-row,
  .allday-row {
    display: grid;
    grid-template-columns: 3.5rem repeat(var(--cols), minmax(0, 1fr));
    border-bottom: 1px solid var(--color-border-default, #262626);
  }
  .day-head {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
    padding: 0.4rem 0.5rem;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .day-head .num {
    font-size: var(--text-sm, 13px);
    font-weight: 600;
    color: var(--color-fg-primary);
    font-variant-numeric: tabular-nums;
  }
  .day-head.today .num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.4rem;
    height: 1.4rem;
    border-radius: var(--radius-chip, 4px);
    background: var(--color-accent, #6366f1);
    color: var(--color-bg-app, #0f0f0f);
  }
  .allday-label {
    padding: 0.25rem 0.5rem;
    font-size: var(--text-2xs, 10px);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .allday-cell {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    min-height: 1.5rem;
    padding: 2px 3px;
    border-inline-start: 1px solid var(--color-border-subtle, #1f1f1f);
    min-width: 0;
  }
  .allday-pill {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 1px 6px;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-accent, #6366f1) 22%, transparent);
    font: inherit;
    font-size: var(--text-2xs, 11px);
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    /* C43: keep programmatic scrolls clear of the sticky chrome above. */
    scroll-padding-top: 2rem;
  }
  .grid {
    position: relative;
    display: grid;
    grid-template-columns: 3.5rem repeat(var(--cols), minmax(0, 1fr));
  }
  .hours {
    position: relative;
  }
  .hour {
    position: absolute;
    right: 0.5rem;
    translate: 0 -50%;
    font-size: var(--text-2xs, 10px);
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .day-col {
    position: relative;
    border-inline-start: 1px solid var(--color-border-subtle, #1f1f1f);
    min-width: 0;
  }
  .day-col.today {
    background: color-mix(in srgb, var(--color-accent, #6366f1) 4%, transparent);
  }
  .line {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 1px solid var(--color-border-subtle, #1f1f1f);
  }
  .now {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 2px solid var(--color-accent, #6366f1);
    z-index: 2;
  }
  .block {
    position: absolute;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: hidden;
    padding: 3px 6px;
    border: none;
    border-inline-start: 2px solid var(--color-accent, #6366f1);
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-accent, #6366f1) 18%, var(--color-bg-card, #171717));
    font: inherit;
    text-align: start;
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .block:focus-visible,
  .allday-pill:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .b-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-xs, 12px);
    font-weight: 500;
  }
  .b-time {
    font-size: var(--text-2xs, 10px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    font-variant-numeric: tabular-nums;
  }
</style>
