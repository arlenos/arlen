<script lang="ts">
  /// Seven small day-of-week pills. Stored as `[0..6]` where 0 = Monday
  /// (matching the daemon's `DndSchedule.days` schema). An empty array
  /// is rendered as "every day" upstream — this picker shows all pills
  /// inactive in that case and lets the user opt in.

  let {
    value,
    onchange,
  }: {
    value: number[];
    onchange: (value: number[]) => void;
  } = $props();

  const DAYS = [
    { idx: 0, label: "Mo" },
    { idx: 1, label: "Tu" },
    { idx: 2, label: "We" },
    { idx: 3, label: "Th" },
    { idx: 4, label: "Fr" },
    { idx: 5, label: "Sa" },
    { idx: 6, label: "Su" },
  ];

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

<div class="days" role="group" aria-label="Days of week">
  {#each DAYS as day}
    {@const active = value.includes(day.idx)}
    <button
      type="button"
      class="day"
      class:active
      aria-pressed={active}
      onclick={() => toggle(day.idx)}
    >
      {day.label}
    </button>
  {/each}
</div>

<style>
  .days {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .day {
    width: 28px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.6875rem;
    font-weight: 600;
    cursor: pointer;
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
