<script lang="ts">
  /// Single-select pill row (action mode, schedule modes, access tier, …).
  /// Canonical replacement for the bespoke `seg-pill` markup re-invented per
  /// page. Bindable `value` + `onchange`, forwards `id` for deep-link/search.
  let {
    options,
    value = $bindable(),
    id,
    disabled = false,
    ariaLabel,
    onchange,
    class: className,
  }: {
    /// The selectable options, in display order.
    options: { value: string; label: string }[];
    /// The currently selected value (bindable).
    value: string;
    /// Optional anchor id for deep-link scroll-to-setting.
    id?: string;
    disabled?: boolean;
    ariaLabel?: string;
    onchange?: (value: string) => void;
    class?: string;
  } = $props();

  function select(v: string) {
    if (disabled || v === value) return;
    value = v;
    onchange?.(v);
  }
</script>

<div class="seg {className ?? ''}" {id} role="radiogroup" aria-label={ariaLabel}>
  {#each options as opt (opt.value)}
    <button
      type="button"
      role="radio"
      aria-checked={value === opt.value}
      {disabled}
      class="seg-pill"
      class:active={value === opt.value}
      onclick={() => select(opt.value)}
    >
      {opt.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .seg-pill {
    appearance: none;
    border: 1px solid transparent;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-size: 0.8125rem;
    font-weight: 500;
    padding: 4px 12px;
    min-height: var(--height-control-compact, 24px);
    border-radius: calc(var(--radius-input) - 2px);
    cursor: pointer;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }

  .seg-pill:hover:not(:disabled):not(.active) {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }

  .seg-pill.active {
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 30%, transparent);
    color: var(--foreground);
  }

  .seg:has(.seg-pill:disabled) {
    opacity: 0.5;
  }
  .seg-pill:disabled {
    cursor: not-allowed;
  }
</style>
