<script lang="ts">
  /// One editable theme field with the shared override language: the control
  /// always shows the resolved active value at full contrast; when the user has
  /// overridden it, an accent bar sits in the left gutter (VS Code's modified
  /// mark), and a reset control appears on hover to fall back to the theme's
  /// value. There is never a greyed placeholder for the theme value.
  import type { Snippet } from "svelte";
  import { RotateCcw } from "lucide-svelte";

  let {
    label,
    hint,
    overridden = false,
    onreset,
    control,
    id,
  }: {
    label: string;
    hint?: string;
    /// True when a per-field override is set (shows the bar + reset).
    overridden?: boolean;
    onreset?: () => void;
    /// The editor for this field (a colour swatch, a slider, a select).
    control?: Snippet;
    id?: string;
  } = $props();
</script>

<div class="or-row" class:overridden {id}>
  <span class="or-bar" aria-hidden="true"></span>
  <div class="or-label">
    <span class="or-title">{label}</span>
    {#if hint}<span class="or-hint">{hint}</span>{/if}
  </div>
  <div class="or-control">
    {#if overridden}
      <button
        type="button"
        class="or-reset"
        title="Reset to the theme's value"
        aria-label={`Reset ${label} to the theme's value`}
        onclick={onreset}
      >
        <RotateCcw size={13} strokeWidth={2} />
      </button>
    {/if}
    {@render control?.()}
  </div>
</div>

<style>
  .or-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 1rem;
  }
  /* The modified mark: an accent bar in the left gutter, only when overridden. */
  .or-bar {
    position: absolute;
    left: 0;
    top: 0.375rem;
    bottom: 0.375rem;
    width: 3px;
    border-radius: var(--radius-full, 9999px);
    background: transparent;
  }
  .or-row.overridden .or-bar {
    background: var(--color-accent, var(--foreground));
  }
  .or-label {
    display: flex;
    flex-direction: column;
    gap: 0.0625rem;
    flex: 1;
    min-width: 0;
  }
  .or-title {
    font-size: var(--text-sm);
    color: var(--foreground);
  }
  .or-hint {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .or-control {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }
  /* Reset shows on row hover, only when the field is overridden. */
  .or-reset {
    display: inline-flex;
    opacity: 0;
    border: none;
    background: transparent;
    padding: 0.25rem;
    border-radius: var(--radius-button, 6px);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    transition:
      opacity 100ms ease,
      color 100ms ease;
  }
  .or-row:hover .or-reset,
  .or-reset:focus-visible {
    opacity: 1;
  }
  .or-reset:hover {
    color: var(--foreground);
  }
</style>
