<script lang="ts">
  /// The time grid: one column per day, an all-day strip pinned on top, timed
  /// events as positioned blocks that split the column when they overlap. Also
  /// the day view - the same engine with one column. Slot height stays >= 24px
  /// per half hour (calendar-app.md §7a c); the initial scroll lands the
  /// working morning in view.
  ///
  /// Interaction: press on empty grid and drag to span a time - release opens
  /// the quick-create; press a block and drag to move it (15-minute snap,
  /// across days), drag its lower edge to resize, hold Alt while dropping to
  /// duplicate. Every drag has a single-pointer alternative (SC 2.5.7): the
  /// block's popover opens the full edit dialog. Dropping a repeating
  /// occurrence asks the three-way series question first - never a silent
  /// claim about the other occurrences.
  import { t, locale } from "$lib/i18n/messages";
  import { isToday } from "$lib/wording";
  import {
    calendars,
    colorOf,
    layoutDay,
    parseYmd,
    updateEvent,
    createEvent,
    ymd,
    type AgendaEvent,
    type EventChanges,
  } from "$lib/stores/calendar";
  import EventPopover from "./EventPopover.svelte";

  let {
    days,
    events,
    onquick,
    onedit,
    onmoverepeat,
  }: {
    /// The visible dates (YYYY-MM-DD), Monday-first for a week, one for a day.
    days: string[];
    events: AgendaEvent[];
    /// Open the quick-create at a spanned slot (screen coords for anchoring).
    onquick: (q: { date: string; time: string; endTime: string; x: number; y: number }) => void;
    /// Open the full edit dialog for one event.
    onedit: (e: AgendaEvent) => void;
    /// A repeating occurrence was dropped somewhere: ask the scope question
    /// before anything is written.
    onmoverepeat: (e: AgendaEvent, changes: EventChanges) => void;
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

  function minToTime(min: number): string {
    const m = Math.max(0, Math.min(24 * 60, min));
    return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
  }
  const snap = (min: number, step: number) => Math.round(min / step) * step;

  let scroller = $state<HTMLElement | null>(null);
  let gridEl = $state<HTMLElement | null>(null);
  $effect(() => {
    // Land the morning in view once the grid exists; 8:00 minus a breath.
    if (scroller) scroller.scrollTop = 7.5 * HOUR;
  });

  /// Which date a clientX falls into, from the grid's own geometry.
  function dateAtX(clientX: number): string {
    if (!gridEl) return days[0];
    const rect = gridEl.getBoundingClientRect();
    const gutter = 3.5 * 16;
    const colW = (rect.width - gutter) / days.length;
    const i = Math.floor((clientX - rect.left - gutter) / colW);
    return days[Math.max(0, Math.min(days.length - 1, i))];
  }
  function minAtY(clientY: number): number {
    if (!gridEl) return 0;
    const rect = gridEl.getBoundingClientRect();
    return ((clientY - rect.top) / HOUR) * 60;
  }

  // --- Spanning a new slot on empty grid -----------------------------------
  let spanning = $state<{ date: string; anchorMin: number; startMin: number; endMin: number } | null>(null);

  function colDown(e: PointerEvent, date: string): void {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".block")) return;
    const m = snap(minAtY(e.clientY), 30);
    spanning = { date, anchorMin: m, startMin: m, endMin: m + 30 };
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      /* a synthetic or already-lifted pointer has no capture to take */
    }
  }
  function colMove(e: PointerEvent): void {
    if (!spanning) return;
    const m = snap(minAtY(e.clientY), 30);
    spanning = {
      ...spanning,
      date: dateAtX(e.clientX),
      startMin: Math.min(spanning.anchorMin, m),
      endMin: Math.max(spanning.anchorMin + 30, m + 30),
    };
  }
  function colUp(e: PointerEvent): void {
    if (!spanning) return;
    const q = spanning;
    spanning = null;
    onquick({
      date: q.date,
      time: minToTime(q.startMin),
      endTime: minToTime(Math.max(q.endMin, q.startMin + 30)),
      x: e.clientX,
      y: e.clientY,
    });
  }

  // --- Moving / resizing an existing block ---------------------------------
  type DragState = {
    event: AgendaEvent;
    mode: "move" | "resize";
    grabOffsetMin: number;
    date: string;
    startMin: number;
    endMin: number;
    moved: boolean;
    duplicate: boolean;
  };
  let drag = $state<DragState | null>(null);

  function blockDown(e: PointerEvent, ev: AgendaEvent, startMin: number, endMin: number): void {
    if (e.button !== 0) return;
    const nearBottom = (endMin - minAtY(e.clientY)) * (HOUR / 60) < 8;
    drag = {
      event: ev,
      mode: nearBottom ? "resize" : "move",
      grabOffsetMin: minAtY(e.clientY) - startMin,
      date: ev.date,
      startMin,
      endMin,
      moved: false,
      duplicate: false,
    };
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      /* a synthetic or already-lifted pointer has no capture to take */
    }
  }
  function blockMove(e: PointerEvent): void {
    if (!drag) return;
    const pointerMin = minAtY(e.clientY);
    if (drag.mode === "move") {
      const len = drag.endMin - drag.startMin;
      const start = Math.max(0, Math.min(24 * 60 - len, snap(pointerMin - drag.grabOffsetMin, 15)));
      drag = {
        ...drag,
        date: dateAtX(e.clientX),
        startMin: start,
        endMin: start + len,
        moved: true,
        duplicate: e.altKey,
      };
    } else {
      const end = Math.max(drag.startMin + 15, snap(pointerMin, 15));
      drag = { ...drag, endMin: Math.min(24 * 60, end), moved: true };
    }
  }
  async function blockUp(e: PointerEvent): Promise<void> {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (!d.moved) return; // a plain click: the popover trigger handles it
    suppressClick = true;
    const changes = {
      date: d.date,
      time: minToTime(d.startMin),
      endTime: minToTime(d.endMin),
    };
    if (d.event.repeats) {
      onmoverepeat(d.event, changes);
      return;
    }
    if (d.duplicate || e.altKey) {
      await createEvent({
        summary: d.event.summary,
        date: d.date,
        allDay: false,
        time: changes.time,
        endTime: changes.endTime,
        location: d.event.location,
        repeat: "none",
        onDays: [],
        calendarId: d.event.calendar ?? "",
        alarms: d.event.alarms ?? [],
      });
    } else {
      await updateEvent(d.event.uid, d.event.calendar ?? "", changes);
    }
  }
  /// After a real drag, the release still fires a click on the block, which
  /// would pop the details open over the drop. Swallowed exactly once.
  let suppressClick = false;
  function maybeSuppress(e: MouseEvent): void {
    if (suppressClick) {
      e.stopPropagation();
      e.preventDefault();
      suppressClick = false;
    }
  }
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
          <EventPopover event={e} {onedit}>
            {#snippet children(props: Record<string, unknown>)}
              <button
                type="button"
                class="allday-pill"
                style="--cal: {colorOf($calendars, e)}"
                {...props}>{e.summary}</button
              >
            {/snippet}
          </EventPopover>
        {/each}
      </span>
    {/each}
  </div>

  <div class="scroll" bind:this={scroller}>
    <div class="grid" style="height: {24 * HOUR}px" bind:this={gridEl}>
      <div class="gutter hours">
        {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
          <span class="hour" style="top: {h * HOUR}px">{String(h).padStart(2, "0")}:00</span>
        {/each}
      </div>
      {#each days as d (d)}
        {@const today = isToday(d, new Date())}
        <!-- The column is a drawing surface for spanning a new event; its
             interactive children are the block buttons. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="day-col"
          class:today
          onpointerdown={(e) => colDown(e, d)}
          onpointermove={colMove}
          onpointerup={colUp}
        >
          {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
            <span class="line" style="top: {h * HOUR}px" aria-hidden="true"></span>
          {/each}
          {#if today && ymd(now) === d}
            <span class="now" style="top: {nowTop}px" aria-hidden="true"></span>
          {/if}
          {#if spanning && spanning.date === d}
            <span
              class="ghost"
              style="top: {(spanning.startMin * HOUR) / 60}px; height: {((spanning.endMin - spanning.startMin) * HOUR) / 60}px"
              aria-hidden="true"
            >
              {minToTime(spanning.startMin)}&#8211;{minToTime(spanning.endMin)}
            </span>
          {/if}
          {#each layoutDay(byDay.get(d) ?? []) as b (b.event.uid + b.event.date + (b.event.time ?? ""))}
            {@const isDragged =
              drag !== null && drag.event.uid === b.event.uid && drag.event.date === b.event.date && !drag.duplicate}
            {@const top = isDragged && drag ? (drag.startMin * HOUR) / 60 : (b.startMin * HOUR) / 60}
            {@const height =
              isDragged && drag
                ? Math.max(((drag.endMin - drag.startMin) * HOUR) / 60, 20)
                : Math.max(((b.endMin - b.startMin) * HOUR) / 60, 20)}
            {@const shownDate = isDragged && drag ? drag.date : d}
            {#if shownDate === d}
              <EventPopover event={b.event} {onedit}>
                {#snippet children(props: Record<string, unknown>)}
                  <button
                    type="button"
                    class="block"
                    class:dragging={isDragged && drag?.moved}
                   
                    style="top: {top}px; height: {height}px; left: calc({(b.col / b.cols) * 100}% + 2px); width: calc({100 / b.cols}% - 4px); --cal: {colorOf($calendars, b.event)}"
                    onpointerdown={(e) => blockDown(e, b.event, b.startMin, b.endMin)}
                    onpointermove={blockMove}
                    onpointerup={blockUp}
                    onclickcapture={maybeSuppress}
                    {...props}
                  >
                    {#if b.endMin - b.startMin < 40}
                      <!-- A short block holds one line, and the line that
                           matters is the name; the time is where it sits. -->
                      <span class="b-title">{b.event.summary}</span>
                    {:else}
                      <span class="b-title">{b.event.summary}</span>
                      <span class="b-time"
                        >{isDragged && drag ? minToTime(drag.startMin) : b.event.time}&#8211;{isDragged && drag
                          ? minToTime(drag.endMin)
                          : (b.event.end_time ?? "")}</span
                      >
                    {/if}
                    <span class="resize-handle" aria-hidden="true"></span>
                  </button>
                {/snippet}
              </EventPopover>
            {/if}
          {/each}
          {#if drag && drag.moved && drag.date === d && (drag.duplicate || drag.event.date !== d)}
            <!-- The drop preview in the target column (a move across days, or
                 an Alt-duplicate) - the original stays where it is. -->
            <span
              class="ghost strong"
              style="top: {(drag.startMin * HOUR) / 60}px; height: {Math.max(((drag.endMin - drag.startMin) * HOUR) / 60, 20)}px; --cal: {colorOf($calendars, drag.event)}"
              aria-hidden="true"
            >
              {drag.event.summary}
            </span>
          {/if}
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
    background: color-mix(in srgb, var(--cal, var(--color-accent, #6366f1)) 26%, transparent);
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
    touch-action: none;
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
  .ghost {
    position: absolute;
    left: 2px;
    right: 2px;
    z-index: 3;
    display: flex;
    align-items: flex-start;
    padding: 3px 6px;
    border: 1px dashed color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-2xs, 10px);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .ghost.strong {
    border-style: solid;
    border-color: var(--cal, var(--color-accent));
    background: color-mix(in srgb, var(--cal, var(--color-accent)) 18%, transparent);
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
    border-inline-start: 2px solid var(--cal, var(--color-accent, #6366f1));
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--cal, var(--color-accent, #6366f1)) 20%, var(--color-bg-card, #171717));
    font: inherit;
    text-align: start;
    color: var(--color-fg-primary);
    cursor: pointer;
    touch-action: none;
  }
  .block.dragging {
    z-index: 4;
    opacity: 0.85;
    cursor: grabbing;
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
  .resize-handle {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 6px;
    cursor: ns-resize;
  }
</style>
