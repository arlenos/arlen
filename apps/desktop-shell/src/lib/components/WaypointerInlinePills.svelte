<script lang="ts">
  /// Compact inline pills (segmented control) for Waypointer settings.
  /// 28px height, fits inside a CommandItem row.

  import type { SelectOption } from "$lib/stores/settingsSearch";

  let {
    value,
    options,
    onchange,
  }: {
    value: string;
    options: SelectOption[];
    onchange: (value: string) => void;
  } = $props();

  function pick(v: string, e: MouseEvent) {
    e.stopPropagation();
    onchange(v);
  }
</script>

<div class="wp-pills" role="radiogroup">
  {#each options as opt}
    {@const active = value === opt.value}
    <button
      type="button"
      role="radio"
      aria-checked={active}
      class="wp-pill"
      class:active
      onclick={(e) => pick(opt.value, e)}
    >
      {opt.label}
    </button>
  {/each}
</div>

<style>
  .wp-pills {
    display: inline-flex;
    gap: 1px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    flex-shrink: 0;
  }
  .wp-pill {
    height: 20px;
    padding: 0 0.5rem;
    border-radius: var(--radius-sm);
    background: transparent;
    border: none;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    font-size: 0.625rem;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: all 100ms ease;
  }
  .wp-pill:hover {
    color: var(--color-fg-shell);
  }
  .wp-pill.active {
    background: color-mix(in srgb, var(--color-accent) 22%, transparent);
    color: var(--color-fg-shell);
  }
</style>
