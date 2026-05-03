<script lang="ts">
  /// Number input with prominent side buttons.
  ///
  /// Canonical source lives in `sdk/ui-kit`; consuming apps keep file
  /// copies under their own `src/lib/components/ui/number-input/`
  /// (see the other shared primitives for why — Tailwind scope hashing
  /// breaks across symlinked components). Sync by copying when this
  /// file changes.
  ///
  /// Design goals:
  ///
  /// * 36×36 minimum hit target on both the `−` and `+` buttons so
  ///   this works on touchpads and touch screens without a zoom pass.
  /// * Native spin-buttons are hidden (they were the reason we built
  ///   this component — they're <12px on most browsers).
  /// * Arrow-Up / Arrow-Down on the focused input still increment
  ///   because we keep `type="number"` and let the browser do it;
  ///   state syncs on `change` (blur / Enter).
  /// * Min/max disable the respective button but still allow direct
  ///   typing; values outside the range are clamped on commit.

  import { Minus, Plus } from "lucide-svelte";

  type Props = {
    value: number;
    min?: number;
    max?: number;
    step?: number;
    /// Shown faintly to the right of the number (e.g. "chars/s", "ms").
    unit?: string;
    disabled?: boolean;
    ariaLabel?: string;
    /// CSS width for the whole control. Defaults to `auto` which sizes
    /// around the content; pass `"180px"` etc. to align multiple rows.
    width?: string;
    onchange: (value: number) => void;
  };

  let {
    value,
    min,
    max,
    step = 1,
    unit,
    disabled = false,
    ariaLabel,
    width,
    onchange,
  }: Props = $props();

  const canDecrement = $derived(
    !disabled && (min === undefined || value - step >= min)
  );
  const canIncrement = $derived(
    !disabled && (max === undefined || value + step <= max)
  );

  function clamp(n: number): number {
    let out = n;
    if (min !== undefined && out < min) out = min;
    if (max !== undefined && out > max) out = max;
    return out;
  }

  function decrement(): void {
    if (!canDecrement) return;
    const next = clamp(value - step);
    if (next !== value) onchange(next);
  }

  function increment(): void {
    if (!canIncrement) return;
    const next = clamp(value + step);
    if (next !== value) onchange(next);
  }

  /// Fires on blur / Enter — not on every keystroke. That lets the
  /// user type `1500` without the value being clamped to `max` in the
  /// middle of typing `1`, `15`, ...
  function handleChange(e: Event): void {
    const raw = (e.target as HTMLInputElement).value;
    const parsed = Number.parseFloat(raw);
    if (Number.isNaN(parsed)) {
      // Snap the DOM back to the last known good value.
      (e.target as HTMLInputElement).value = String(value);
      return;
    }
    const clamped = clamp(parsed);
    if (clamped !== value) {
      onchange(clamped);
    } else {
      // Parsed value equalled current — still re-sync the DOM in
      // case the user typed whitespace or a leading zero.
      (e.target as HTMLInputElement).value = String(value);
    }
  }
</script>

<div class="wrap" class:disabled style={width ? `width: ${width};` : ""}>
  <button
    type="button"
    class="btn"
    onclick={decrement}
    disabled={!canDecrement}
    aria-label={ariaLabel ? `Decrease ${ariaLabel}` : "Decrease"}
  >
    <Minus size={14} strokeWidth={2.25} />
  </button>
  <div class="field">
    <input
      type="number"
      {value}
      {min}
      {max}
      {step}
      {disabled}
      aria-label={ariaLabel}
      onchange={handleChange}
    />
    {#if unit}
      <span class="unit">{unit}</span>
    {/if}
  </div>
  <button
    type="button"
    class="btn"
    onclick={increment}
    disabled={!canIncrement}
    aria-label={ariaLabel ? `Increase ${ariaLabel}` : "Increase"}
  >
    <Plus size={14} strokeWidth={2.25} />
  </button>
</div>

<style>
  .wrap {
    display: inline-flex;
    align-items: stretch;
    border-radius: var(--radius-input);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 12%, transparent);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    overflow: hidden;
    /* Inner buttons share a divider using `:not(:first-child)` so we
     * don't need two distinct button classes. */
  }

  .wrap.disabled {
    opacity: 0.5;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    min-width: 36px;
    height: var(--control-h);
    background: transparent;
    border: none;
    color: var(--foreground);
    cursor: pointer;
    padding: 0;
    transition: background-color 120ms ease;
  }

  .btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .btn:active:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 16%, transparent);
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .btn + .field {
    border-left: 1px solid
      color-mix(in srgb, var(--foreground) 12%, transparent);
  }
  .field + .btn {
    border-left: 1px solid
      color-mix(in srgb, var(--foreground) 12%, transparent);
  }

  .field {
    /* Center both the input and the unit vertically against the
     * 36px button height. `align-items: center` keeps them on the
     * same optical midline; baseline would pull the input upwards
     * because <input>'s baseline sits above its box centre. */
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 0 10px;
    height: var(--control-h);
    min-width: 72px;
    flex: 1;
  }

  .field input {
    /* The spin-buttons in Chromium/Firefox are tiny — hide them and
     * let our side buttons do the work. `appearance: textfield` is
     * still needed on Firefox to kill the reserved right-edge gutter.
     */
    -moz-appearance: textfield;
    appearance: textfield;
    width: 100%;
    min-width: 40px;
    height: var(--control-h);
    background: transparent;
    border: none;
    outline: none;
    font: inherit;
    font-size: 14px;
    line-height: var(--control-h);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
    text-align: center;
    color: var(--foreground);
    padding: 0;
  }

  .field input::-webkit-inner-spin-button,
  .field input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .field input:focus-visible {
    outline: none;
  }

  .unit {
    font-size: 13px;
    line-height: 1;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    white-space: nowrap;
    pointer-events: none;
  }
</style>
