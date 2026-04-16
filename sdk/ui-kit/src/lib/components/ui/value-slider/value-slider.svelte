<script lang="ts">
  /// Compact slider with value pill. Fits inline in a Row.

  let {
    value,
    min = 0,
    max = 100,
    step = 1,
    unit = "",
    ariaLabel,
    onchange,
  }: {
    value: number;
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
    ariaLabel?: string;
    onchange: (v: number) => void;
  } = $props();

  const percent = $derived(((value - min) / (max - min)) * 100);

  function onInput(e: Event) {
    const v = parseFloat((e.currentTarget as HTMLInputElement).value);
    onchange(v);
  }
</script>

<div class="wrap">
  <div class="track-wrap" style="--percent: {percent}%">
    <div class="track"></div>
    <div class="track-fill"></div>
    <div class="thumb"></div>
    <input
      type="range"
      {min}
      {max}
      {step}
      {value}
      oninput={onInput}
      aria-label={ariaLabel}
    />
  </div>
  <div class="value-pill">
    <span>{value}</span>
    {#if unit}<span class="unit">{unit}</span>{/if}
  </div>
</div>

<style>
  .wrap {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    width: 200px;
  }

  .track-wrap {
    position: relative;
    flex: 1;
    height: 18px;
    display: flex;
    align-items: center;
  }

  .track {
    position: absolute;
    left: 0;
    right: 0;
    height: 3px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .track-fill {
    position: absolute;
    left: 0;
    width: var(--percent);
    height: 3px;
    border-radius: var(--radius-sm);
    background: var(--color-accent);
  }

  .thumb {
    position: absolute;
    left: var(--percent);
    width: 12px;
    height: 12px;
    margin-left: -6px;
    border-radius: var(--radius-md);
    background: var(--foreground);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.4),
      0 0 0 0 color-mix(in srgb, var(--color-accent) 40%, transparent);
    transition: box-shadow 150ms ease;
    pointer-events: none;
  }

  .track-wrap:hover .thumb,
  .track-wrap:focus-within .thumb {
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.4),
      0 0 0 3px color-mix(in srgb, var(--color-accent) 20%, transparent);
  }

  input[type="range"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0;
    opacity: 0;
    cursor: pointer;
    appearance: none;
    -webkit-appearance: none;
  }

  .value-pill {
    display: inline-flex;
    align-items: baseline;
    justify-content: flex-end;
    gap: 2px;
    min-width: 40px;
    font-size: 0.6875rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    white-space: nowrap;
    text-align: right;
  }

  .unit {
    font-size: 0.625rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
