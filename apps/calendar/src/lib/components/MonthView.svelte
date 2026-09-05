<script lang="ts">
  /// The month: six weeks of cells, Monday first, each row led by its ISO
  /// week number. A cell shows up to three event pills and counts the rest
  /// out loud; the count is a button that hands the day to the day view, so
  /// nothing is reachable only by squinting. A pill drags to another day
  /// (the time stays); the click path through the popover and the dialog is
  /// the single-pointer alternative SC 2.5.7 asks for.
  import { t, locale } from "$lib/i18n/messages";
  import { isToday, isoWeek } from "$lib/wording";
  import {
    addDays,
    calendars,
    colorOf,
    parseYmd,
    startOfWeek,
    updateEvent,
    type AgendaEvent,
    type EventChanges,
  } from "$lib/stores/calendar";
  import EventPopover from "./EventPopover.svelte";

  let {
    month,
    events,
    onopenday,
    onedit,
    onmoverepeat,
  }: {
    /// Any date inside the month to show (YYYY-MM-DD).
    month: string;
    events: AgendaEvent[];
    onopenday: (date: string) => void;
    onedit?: (e: AgendaEvent) => void;
    /// A repeating occurrence was dropped on another day: ask the scope
    /// question before anything is written.
    onmoverepeat?: (e: AgendaEvent, changes: EventChanges) => void;
  } = $props();

  const first = $derived(`${month.slice(0, 7)}-01`);
  /// Six rows of seven, each with the week number its Monday falls in.
  const rows = $derived.by(() => {
    const start = startOfWeek(first);
    return Array.from({ length: 6 }, (_, r) => {
      const monday = addDays(start, r * 7);
      return { monday, week: isoWeek(monday), days: Array.from({ length: 7 }, (_, i) => addDays(monday, i)) };
    });
  });

  const byDay = $derived.by(() => {
    const map = new Map<string, AgendaEvent[]>();
    for (const e of events) {
      const list = map.get(e.date);
      if (list) list.push(e);
      else map.set(e.date, [e]);
    }
    return map;
  });

  const dowNames = $derived.by(() => {
    const fmt = new Intl.DateTimeFormat($locale, { weekday: "short", timeZone: "UTC" });
    return Array.from({ length: 7 }, (_, i) => fmt.format(new Date(Date.UTC(2024, 0, 1 + i))));
  });

  const MAX_PILLS = 3;

  // --- Dragging a pill to another day ---------------------------------------
  // The pointer is captured by the pill, so the cell under it is found by
  // hit-testing the point rather than by pointer events on the cells.
  let drag = $state<{ event: AgendaEvent; over: string | null; moved: boolean; x0: number; y0: number } | null>(null);

  function pillDown(e: PointerEvent, ev: AgendaEvent): void {
    if (e.button !== 0) return;
    drag = { event: ev, over: null, moved: false, x0: e.clientX, y0: e.clientY };
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      /* a synthetic or already-lifted pointer has no capture to take */
    }
  }
  function pillMove(e: PointerEvent): void {
    if (!drag) return;
    if (!drag.moved && Math.hypot(e.clientX - drag.x0, e.clientY - drag.y0) < 4) return;
    const cell = document.elementFromPoint(e.clientX, e.clientY)?.closest(".cell") as HTMLElement | null;
    drag = { ...drag, moved: true, over: cell?.dataset.date ?? drag.over };
  }
  async function pillUp(): Promise<void> {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (!d.moved) return; // a plain click: the popover trigger handles it
    suppressClick = true;
    if (!d.over || d.over === d.event.date) return;
    const changes: EventChanges = { date: d.over };
    if (d.event.repeats && onmoverepeat) {
      onmoverepeat(d.event, changes);
      return;
    }
    await updateEvent(d.event.uid, d.event.calendar ?? "", changes, "this", d.event.date);
  }
  /// After a real drag, the release still fires a click on the pill, which
  /// would pop the details open over the drop. Swallowed exactly once.
  let suppressClick = false;
  function maybeSuppress(e: MouseEvent): void {
    if (suppressClick) {
      e.stopPropagation();
      e.preventDefault();
      suppressClick = false;
    }
  }
  const dropTarget = $derived(drag?.moved && drag.over !== drag.event.date ? drag.over : null);
</script>

<div class="month">
  <div class="dow-row">
    <span class="wk-head" aria-hidden="true"></span>
    {#each dowNames as n, i (i)}
      <span class="dow">{n}</span>
    {/each}
  </div>
  <div class="cells">
    {#each rows as row (row.monday)}
      <span class="wk" aria-label={$t("cal.weekN", { n: row.week })}>{row.week}</span>
      {#each row.days as d (d)}
        {@const list = byDay.get(d) ?? []}
        <div
          class="cell"
          class:other={d.slice(0, 7) !== month.slice(0, 7)}
          class:today={isToday(d, new Date())}
          class:drop={dropTarget === d}
          data-date={d}
        >
          <button type="button" class="num" onclick={() => onopenday(d)}>{parseYmd(d).getDate()}</button>
          <div class="pills">
            {#each list.slice(0, MAX_PILLS) as e (e.uid + e.date + (e.time ?? ""))}
              <EventPopover event={e} {onedit}>
                {#snippet children(props: Record<string, unknown>)}
                  <button
                    type="button"
                    class="pill"
                    class:timed={e.time !== null}
                    class:dragging={drag?.moved && drag.event === e}
                    style="--cal: {colorOf($calendars, e)}"
                    onpointerdown={(ev) => pillDown(ev, e)}
                    onpointermove={pillMove}
                    onpointerup={pillUp}
                    onclickcapture={maybeSuppress}
                    {...props}
                  >
                    {#if e.time}<span class="p-time">{e.time}</span>{/if}
                    <span class="p-title">{e.summary}</span>
                  </button>
                {/snippet}
              </EventPopover>
            {/each}
            {#if dropTarget === d && drag}
              <!-- The pill as it will land: same colour, same time, here. -->
              <span class="pill ghost" style="--cal: {colorOf($calendars, drag.event)}" aria-hidden="true">
                {#if drag.event.time}<span class="p-time">{drag.event.time}</span>{/if}
                <span class="p-title">{drag.event.summary}</span>
              </span>
            {/if}
            {#if list.length > MAX_PILLS}
              <button type="button" class="more" onclick={() => onopenday(d)}>
                {$t("cal.more", { n: list.length - MAX_PILLS })}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    {/each}
  </div>
</div>

<style>
  .month {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  /* One narrow column for the week number, then the seven days. */
  .dow-row {
    display: grid;
    grid-template-columns: 2rem repeat(7, minmax(0, 1fr));
    border-bottom: 1px solid var(--color-border-default, #262626);
  }
  .dow {
    padding: 0.35rem 0.5rem;
    font-size: var(--text-2xs, 11px);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .cells {
    display: grid;
    flex: 1;
    min-height: 0;
    grid-template-columns: 2rem repeat(7, minmax(0, 1fr));
    grid-auto-rows: minmax(0, 1fr);
  }
  /* The week number sits where the day numbers sit, one shade quieter than
     a neighbour-month day: a coordinate, not content. */
  .wk {
    padding: 4px 0 0;
    border-bottom: 1px solid var(--color-border-subtle, #1f1f1f);
    text-align: center;
    font-size: var(--text-2xs, 11px);
    /* The secondary token, not a fraction of the primary one: at 35% over the
       grid this is the serious contrast violation axe reports twenty-nine times,
       once per week label in the month and the mini month. Secondary is still
       quieter than a day number, so the hierarchy the grid needs survives. */
    color: var(--color-fg-secondary, #a3a3a3);
    font-variant-numeric: tabular-nums;
  }
  .cell {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    min-height: 0;
    padding: 3px;
    border-inline-start: 1px solid var(--color-border-subtle, #1f1f1f);
    border-bottom: 1px solid var(--color-border-subtle, #1f1f1f);
    overflow: hidden;
  }
  .cell.other {
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .cell.other .num {
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .cell.drop {
    background: color-mix(in srgb, var(--color-accent, #6366f1) 8%, transparent);
  }
  .num {
    align-self: flex-start;
    padding: 1px 5px;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs, 12px);
    font-weight: 500;
    color: var(--color-fg-primary);
    font-variant-numeric: tabular-nums;
  }
  .num:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .cell.today .num {
    background: var(--color-accent, #6366f1);
    color: var(--color-bg-app, #0f0f0f);
    font-weight: 600;
  }
  .pills {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow: hidden;
    container: monthcell / inline-size;
  }
  .pill {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    min-width: 0;
    overflow: hidden;
    padding: 1px 5px;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--cal, var(--color-accent, #6366f1)) 26%, transparent);
    font: inherit;
    font-size: var(--text-2xs, 11px);
    text-align: start;
    color: var(--color-fg-primary);
    touch-action: none;
  }
  .pill.dragging {
    opacity: 0.35;
  }
  .pill.ghost {
    border: 1px dashed color-mix(in srgb, var(--cal, var(--color-accent)) 60%, transparent);
    background: color-mix(in srgb, var(--cal, var(--color-accent, #6366f1)) 14%, transparent);
    padding: 0 4px;
  }
  .pill:focus-visible,
  .num:focus-visible,
  .more:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .p-time {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
  }
  /* IN A NARROW MONTH THE NAME IS WHAT THE PILL IS FOR. The time refuses to
     shrink, so in a 55px cell - measured at 720 in German - it took the whole
     pill and every event read `09:00 S`, `14:00 P`, `08:15 D`. Five pills in a
     row saying the same minute and one letter each is a grid that tells you
     nothing at a glance, which is the one thing a month view is for.
     Below six rem the time goes and the name stays whole; at 1280 the cell is
     135px and both fit, so nothing changes there. Dropping it beats shrinking
     it: a time cut to `09:` is a worse answer than no time. */
  @container monthcell (max-width: 6rem) {
    .p-time {
      display: none;
    }
  }
  .p-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    align-self: flex-start;
    padding: 0 5px;
    border: none;
    background: transparent;
    font: inherit;
    font-size: var(--text-2xs, 11px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .more:hover {
    color: var(--color-fg-primary);
  }
</style>
