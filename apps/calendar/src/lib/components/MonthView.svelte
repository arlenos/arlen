<script lang="ts">
  /// The month: six weeks of cells, Monday first. A cell shows up to three
  /// event pills and counts the rest out loud; the count is a button that
  /// hands the day to the day view, so nothing is reachable only by squinting.
  import { t, locale } from "$lib/i18n/messages";
  import { isToday } from "$lib/wording";
  import { addDays, calendars, colorOf, parseYmd, startOfWeek, type AgendaEvent } from "$lib/stores/calendar";
  import EventPopover from "./EventPopover.svelte";

  let {
    month,
    events,
    onopenday,
  }: {
    /// Any date inside the month to show (YYYY-MM-DD).
    month: string;
    events: AgendaEvent[];
    onopenday: (date: string) => void;
  } = $props();

  const first = $derived(`${month.slice(0, 7)}-01`);
  const cells = $derived.by(() => {
    const start = startOfWeek(first);
    return Array.from({ length: 42 }, (_, i) => addDays(start, i));
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
</script>

<div class="month">
  <div class="dow-row">
    {#each dowNames as n, i (i)}
      <span class="dow">{n}</span>
    {/each}
  </div>
  <div class="cells">
    {#each cells as d (d)}
      {@const list = byDay.get(d) ?? []}
      <div class="cell" class:other={d.slice(0, 7) !== month.slice(0, 7)} class:today={isToday(d, new Date())}>
        <button type="button" class="num" onclick={() => onopenday(d)}>{parseYmd(d).getDate()}</button>
        <div class="pills">
          {#each list.slice(0, MAX_PILLS) as e (e.uid + e.date + (e.time ?? ""))}
            <EventPopover event={e}>
              {#snippet children(props: Record<string, unknown>)}
                <button type="button" class="pill" class:timed={e.time !== null} style="--cal: {colorOf($calendars, e)}" {...props}>
                  {#if e.time}<span class="p-time">{e.time}</span>{/if}
                  <span class="p-title">{e.summary}</span>
                </button>
              {/snippet}
            </EventPopover>
          {/each}
          {#if list.length > MAX_PILLS}
            <button type="button" class="more" onclick={() => onopenday(d)}>
              {$t("cal.more", { n: list.length - MAX_PILLS })}
            </button>
          {/if}
        </div>
      </div>
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
  .dow-row {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    border-bottom: 1px solid var(--color-border-default, #262626);
  }
  .dow {
    padding: 0.35rem 0.5rem;
    font-size: var(--text-2xs, 11px);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .cells {
    display: grid;
    flex: 1;
    min-height: 0;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    grid-auto-rows: minmax(0, 1fr);
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
    color: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
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
    cursor: pointer;
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
    cursor: pointer;
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
    cursor: pointer;
  }
  .more:hover {
    color: var(--color-fg-primary);
  }
</style>
