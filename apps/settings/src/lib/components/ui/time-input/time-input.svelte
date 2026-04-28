<script lang="ts">
  /// Compact HH:MM input. Wraps a native `<input type="time">` so the
  /// platform picker stays accessible, but reskins it to fit a Row.

  let {
    value,
    ariaLabel,
    onchange,
  }: {
    value: string;
    ariaLabel?: string;
    onchange: (value: string) => void;
  } = $props();

  function handleInput(e: Event) {
    onchange((e.currentTarget as HTMLInputElement).value);
  }
</script>

<input
  type="time"
  class="time"
  {value}
  aria-label={ariaLabel}
  oninput={handleInput}
/>

<style>
  .time {
    height: var(--control-h);
    width: 96px;
    padding: 0 0.5rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    font: inherit;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    transition:
      background-color 150ms ease,
      border-color 150ms ease;
  }
  .time:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .time:focus-visible {
    outline: none;
    border-color: color-mix(in srgb, var(--color-accent) 50%, transparent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 20%, transparent);
  }
  /* Reskin native control widgets where supported. */
  .time::-webkit-calendar-picker-indicator {
    filter: invert(0.6);
    cursor: pointer;
  }
</style>
