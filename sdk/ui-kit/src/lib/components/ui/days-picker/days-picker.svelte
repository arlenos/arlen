<script lang="ts">
  /// Seven small day-of-week pills. Stored as `[0..6]` where 0 = Monday
  /// (matching the daemon's `DndSchedule.days` schema). An empty array
  /// is rendered as "every day" upstream — this picker shows all pills
  /// inactive in that case and lets the user opt in.
  ///
  /// The names come from `Intl.DateTimeFormat`, not from the catalog. Weekday
  /// abbreviations are calendar data every runtime already carries for every
  /// locale, so translating them by hand would mean re-entering CLDR one
  /// language at a time and getting it wrong in the ones nobody reviews.

  import { locale } from "../../../i18n";
  import { kt } from "../../../i18n/messages.kit";

  let {
    value,
    onchange,
  }: {
    value: number[];
    onchange: (value: number[]) => void;
  } = $props();

  /// 2024-01-01 was a Monday, so day `idx` is that date plus `idx`. Formatting in
  /// UTC keeps it a Monday west of Greenwich too.
  function dayNames(loc: string, style: "short" | "long"): string[] {
    const fmt = new Intl.DateTimeFormat(loc, { weekday: style, timeZone: "UTC" });
    return Array.from({ length: 7 }, (_, i) => fmt.format(Date.UTC(2024, 0, 1 + i)));
  }

  // The pill shows the abbreviation; the accessible name is the full weekday,
  // because "Mo" read aloud is not a day of the week.
  const short = $derived(dayNames($locale, "short"));
  const long = $derived(dayNames($locale, "long"));

  function toggle(idx: number) {
    const set = new Set(value);
    if (set.has(idx)) {
      set.delete(idx);
    } else {
      set.add(idx);
    }
    onchange([...set].sort((a, b) => a - b));
  }
</script>

<div class="days" role="group" aria-label={$kt("k.days.group")}>
  {#each short as label, idx}
    {@const active = value.includes(idx)}
    <button
      type="button"
      class="day"
      class:active
      aria-pressed={active}
      aria-label={long[idx]}
      onclick={() => toggle(idx)}
    >
      {label}
    </button>
  {/each}
</div>

<style>
  /* Arabic has no distinct short weekday form, so its pills carry the full name
     and seven of them do not fit a settings row. Wrapping is the graceful answer;
     in a two-character locale the wrap never triggers. */
  .days {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }

  /* Square while the abbreviation is two characters, growing sideways where it is
     not: German gives "Mo", English "Mon", and a fixed width would clip one of
     them whichever it was cut to fit. */
  .day {
    min-width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    padding: 0 5px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: var(--text-2xs);
    font-weight: 600;
    transition:
      background-color 120ms ease,
      border-color 120ms ease,
      color 120ms ease;
  }
  .day:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
    color: var(--foreground);
  }
  .day.active {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 35%, transparent);
    color: var(--foreground);
  }
</style>
