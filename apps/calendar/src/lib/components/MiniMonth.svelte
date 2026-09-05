<script lang="ts">
  /// The sidebar's month instrument: a small grid to stand somewhere else in
  /// time. Click picks a focus date, chevrons page the month, today wears the
  /// accent, days that hold events carry a dot, each row is led by its ISO
  /// week number. Purely navigational - the big views do the showing.
  import { ChevronLeft, ChevronRight } from "@lucide/svelte";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { t, locale } from "$lib/i18n/messages";
  import { isoWeek, monthTitle } from "$lib/wording";
  import { addDays, parseYmd, startOfWeek, ymd } from "$lib/stores/calendar";

  let {
    focus,
    marked,
    onpick,
  }: {
    focus: string;
    /// Dates (YYYY-MM-DD) that hold at least one event.
    marked: Set<string>;
    onpick: (date: string) => void;
  } = $props();

  /// The first of the month currently shown; follows the focus until the
  /// chevrons detach it.
  // svelte-ignore state_referenced_locally
  let shown = $state(`${focus.slice(0, 7)}-01`);
  $effect(() => {
    shown = `${focus.slice(0, 7)}-01`;
  });

  const today = ymd(new Date());

  function page(n: number): void {
    const d = parseYmd(shown);
    d.setMonth(d.getMonth() + n);
    shown = `${ymd(d).slice(0, 7)}-01`;
  }

  /// Six rows of seven, Monday first, spanning the shown month, each with
  /// the week number its Monday falls in.
  const rows = $derived.by(() => {
    const first = startOfWeek(shown);
    return Array.from({ length: 6 }, (_, r) => {
      const monday = addDays(first, r * 7);
      return { monday, week: isoWeek(monday), days: Array.from({ length: 7 }, (_, i) => addDays(monday, i)) };
    });
  });

  const dayLetters = $derived.by(() => {
    const fmt = new Intl.DateTimeFormat($locale, { weekday: "narrow", timeZone: "UTC" });
    // 2024-01-01 was a Monday; formatting in UTC keeps it one everywhere.
    return Array.from({ length: 7 }, (_, i) => fmt.format(new Date(Date.UTC(2024, 0, 1 + i))));
  });
</script>

<div class="mini">
  <div class="mini-head">
    <span class="mini-title">{monthTitle(shown, $locale)}</span>
    <span class="mini-nav">
      <IconAction label={$t("cal.prev")} onclick={() => page(-1)}>
        <ChevronLeft size={14} strokeWidth={2} />
      </IconAction>
      <IconAction label={$t("cal.next")} onclick={() => page(1)}>
        <ChevronRight size={14} strokeWidth={2} />
      </IconAction>
    </span>
  </div>
  <div class="mini-grid" role="presentation">
    <span class="mini-wk" aria-hidden="true"></span>
    {#each dayLetters as l, i (i)}
      <span class="mini-dow">{l}</span>
    {/each}
    {#each rows as row (row.monday)}
      <span class="mini-wk" aria-label={$t("cal.weekN", { n: row.week })}>{row.week}</span>
      {#each row.days as d (d)}
        <button
          type="button"
          class="mini-day"
          class:other={d.slice(0, 7) !== shown.slice(0, 7)}
          class:today={d === today}
          class:focus={d === focus}
          onclick={() => onpick(d)}
        >
          {parseYmd(d).getDate()}
          {#if marked.has(d)}<span class="mark" aria-hidden="true"></span>{/if}
        </button>
      {/each}
    {/each}
  </div>
</div>

<style>
  .mini {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.5rem 0.5rem 0.25rem;
  }
  .mini-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-inline-start: 0.25rem;
  }
  .mini-title {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
  }
  .mini-nav {
    display: flex;
  }
  /* The week column is narrower than a day and reads as the axis it is. */
  .mini-grid {
    display: grid;
    grid-template-columns: 1.1rem repeat(7, 1fr);
    gap: 1px;
  }
  .mini-dow {
    text-align: center;
    font-size: var(--text-2xs, 10px);
    color: var(--color-fg-secondary, #a3a3a3);
    padding-bottom: 2px;
  }
  .mini-wk {
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    /* Same reason as the month grid's, and more so: nine pixels is the smallest
       text in the app. */
    color: var(--color-fg-secondary, #a3a3a3);
    font-variant-numeric: tabular-nums;
  }
  .mini-day {
    position: relative;
    aspect-ratio: 1;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font: inherit;
    font-size: var(--text-2xs, 11px);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .mini-day:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .mini-day.other {
    /* A day outside the shown month is quieter, not unreadable: these are real
       days somebody can click to walk into that month, and at 30% they were the
       contrast violation left once the week labels were fixed - a second fault
       standing behind the first. Secondary is still below the 80% an in-month
       day carries, so the month being shown still reads as the foreground. */
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .mini-day.focus {
    background: color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
  }
  .mini-day.today {
    background: var(--color-accent, #6366f1);
    color: var(--color-bg-app, #0f0f0f);
    font-weight: 600;
  }
  .mark {
    position: absolute;
    bottom: 2px;
    left: 50%;
    translate: -50% 0;
    width: 3px;
    height: 3px;
    border-radius: var(--radius-full, 9999px);
    background: currentColor;
    opacity: 0.7;
  }
</style>
