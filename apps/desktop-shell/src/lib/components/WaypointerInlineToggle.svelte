<script lang="ts">
  /// Compact inline toggle for Waypointer settings results.
  /// 24px wide, fits inside a CommandItem row.

  let {
    checked,
    onchange,
  }: {
    checked: boolean;
    onchange: (value: boolean) => void;
  } = $props();

  function toggle(e: MouseEvent) {
    e.stopPropagation();
    onchange(!checked);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<button
  type="button"
  role="switch"
  aria-checked={checked}
  class="wp-toggle"
  class:on={checked}
  onclick={toggle}
>
  <span class="wp-toggle-thumb"></span>
</button>

<style>
  .wp-toggle {
    position: relative;
    width: 28px;
    height: 16px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border: none;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: background-color 150ms ease;
  }
  .wp-toggle:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 28%, transparent);
  }
  .wp-toggle.on {
    background: var(--color-accent);
  }
  .wp-toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: var(--radius-sm);
    background: var(--color-fg-shell);
    transition: transform 150ms cubic-bezier(0.4, 0, 0.2, 1);
  }
  .wp-toggle.on .wp-toggle-thumb {
    transform: translateX(12px);
    background: var(--color-bg-shell);
  }
</style>
