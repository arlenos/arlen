<script lang="ts">
  /// A single capability-scope chip: a reach label with an optional revoke
  /// control. Display-only by default; pass `onRevoke` to show the remove
  /// affordance. Unlike `ChipList` there is deliberately no add field - a
  /// capability browser can only ever take reach away, never grant it, so the
  /// chip carries a remove control and nothing else.
  import { X } from "@lucide/svelte";

  let {
    label,
    onRevoke,
    id,
  }: {
    label: string;
    /// When set, renders a remove control that calls back. The caller owns the
    /// confirm step and the actual revoke; this chip only signals intent.
    onRevoke?: () => void;
    id?: string;
  } = $props();
</script>

<span class="scope-chip" class:has-x={!!onRevoke} {id}>
  <span class="sc-label">{label}</span>
  {#if onRevoke}
    <button
      type="button"
      class="sc-x"
      aria-label={`Remove ${label}`}
      onclick={onRevoke}
    >
      <X size={12} strokeWidth={2.5} />
    </button>
  {/if}
</span>

<style>
  .scope-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    height: 1.5rem;
    padding: 0 0.5rem;
    /* Chips ride the roundness scale (radius-chip), not the categorical pill. */
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: 0.75rem;
    line-height: 1;
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .scope-chip.has-x {
    padding-right: 0.1875rem;
  }
  .sc-label {
    white-space: nowrap;
  }
  .sc-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1rem;
    height: 1rem;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
    transition:
      background-color var(--duration-micro, 100ms) var(--ease-out, ease),
      color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .sc-x:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
  }
</style>
