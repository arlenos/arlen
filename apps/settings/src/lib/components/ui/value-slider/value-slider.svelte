<script lang="ts">
  /// Compact slider with a value pill, sized to fit inline in a Row.
  ///
  /// Thin wrapper over the canonical `FillSlider` that adds the pill
  /// + unit suffix used across Settings (radius, brightness, font
  /// size, ...). Settings pages keep the existing call contract
  /// (`value`, `unit`, `onchange`); the bar look comes from the
  /// shared component so future tweaks land in one place.

  import { FillSlider } from "$lib/components/ui/fill-slider";

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
</script>

<div class="wrap">
  <div class="slider-wrap">
    <FillSlider
      {value}
      {min}
      {max}
      {step}
      {ariaLabel}
      oninput={onchange}
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

  .slider-wrap {
    flex: 1;
    display: flex;
    align-items: center;
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
