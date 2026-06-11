<script lang="ts">
  /// Quick Settings slider tile (sound, brightness).
  ///
  /// Composes BaseTile for the whole tile chrome (head, status
  /// strip, detail affordances) and FillSlider for the control —
  /// this file only wires the two together and renders the percent
  /// readout in the head's trailing slot. Always a 2x1 cell.
  import type { Snippet } from "svelte";
  import BaseTile from "./BaseTile.svelte";
  import { FillSlider } from "../ui/fill-slider";

  let {
    label,
    statusText = "",
    icon,
    value,
    min = 0,
    max = 100,
    step = 1,
    disabled = false,
    oninput,
    onfocus,
    onblur,
    oncontextmenu,
    onDetail,
    detailLabel = "",
  }: {
    /// Tile label, top-left next to the icon.
    label: string;
    /// Status strip line (e.g. the active output device).
    statusText?: string;
    icon?: Snippet;
    value: number;
    min?: number;
    max?: number;
    step?: number;
    disabled?: boolean;
    oninput?: (value: number) => void;
    onfocus?: () => void;
    onblur?: () => void;
    oncontextmenu?: () => void;
    onDetail?: () => void;
    detailLabel?: string;
  } = $props();

  /// Clamped like FillSlider's own fill math, so an out-of-range
  /// backend value can't render a -5% or 120% readout next to a
  /// clamped bar.
  const percent = $derived(
    Math.max(0, Math.min(100, ((value - min) / (max - min)) * 100)),
  );
</script>

<BaseTile
  {label}
  {statusText}
  {icon}
  size="2x1"
  {disabled}
  {oncontextmenu}
  {onDetail}
  {detailLabel}
  tabindex={-1}
>
  {#snippet headTrailing()}
    {Math.round(percent)}%
  {/snippet}

  <div class="qs-slider-row">
    <FillSlider
      {value}
      {min}
      {max}
      {step}
      {disabled}
      ariaLabel={label}
      {oninput}
      {onfocus}
      {onblur}
    />
  </div>
</BaseTile>

<style>
  .qs-slider-row {
    display: flex;
    flex-direction: column;
    padding-bottom: 4px;
  }
</style>
