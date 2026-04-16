<script lang="ts">
  /// Visual 3x3 grid showing the five valid toast anchor corners.
  /// Center column only has a top-center slot; bottom-center is
  /// intentionally omitted because svelte-sonner does not anchor
  /// there and it would be confusing.

  type Position =
    | "top-left"
    | "top-center"
    | "top-right"
    | "bottom-left"
    | "bottom-right";

  let {
    value,
    onchange,
  }: {
    value: Position;
    onchange: (value: Position) => void;
  } = $props();

  /// 3x3 layout. `null` slots are blanks. `disabled` slots render but
  /// do nothing (used to communicate that the position exists in the
  /// grid even though it isn't a valid anchor).
  const SLOTS: (Position | null)[] = [
    "top-left",
    "top-center",
    "top-right",
    null,
    null,
    null,
    "bottom-left",
    null,
    "bottom-right",
  ];
</script>

<div class="grid" role="radiogroup" aria-label="Toast position">
  {#each SLOTS as slot, i (i)}
    {#if slot}
      {@const selected = value === slot}
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        aria-label={slot}
        class="slot"
        class:selected
        onclick={() => onchange(slot)}
      >
        <span class="dot"></span>
      </button>
    {:else}
      <span class="slot blank" aria-hidden="true"></span>
    {/if}
  {/each}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    grid-template-rows: repeat(3, 1fr);
    gap: 3px;
    width: 80px;
    height: 56px;
    padding: 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .slot {
    border-radius: var(--radius-sm);
    background: transparent;
    border: 1px solid transparent;
    padding: 0;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      background-color 120ms ease,
      border-color 120ms ease;
  }
  .slot.blank {
    pointer-events: none;
  }
  .slot:hover:not(.blank):not(.selected) {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .slot.selected {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 40%, transparent);
  }

  .dot {
    width: 8px;
    height: 4px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 35%, transparent);
    transition: background-color 120ms ease;
  }
  .slot.selected .dot {
    background: var(--color-accent);
  }
</style>
