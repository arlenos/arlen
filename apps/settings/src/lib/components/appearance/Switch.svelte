<script lang="ts">
  /// Compact on/off switch, sized to fit in an Appearance Row.

  let {
    value,
    ariaLabel,
    onchange,
  }: {
    value: boolean;
    ariaLabel?: string;
    onchange: (value: boolean) => void;
  } = $props();

  function toggle() {
    onchange(!value);
  }
</script>

<button
  type="button"
  role="switch"
  aria-checked={value}
  aria-label={ariaLabel}
  class="sw"
  class:on={value}
  onclick={toggle}
>
  <span class="thumb"></span>
</button>

<style>
  .sw {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 999px;
    border: 1px solid
      color-mix(in srgb, var(--foreground) 14%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    cursor: pointer;
    padding: 0;
    transition:
      background-color 150ms ease,
      border-color 150ms ease;
  }

  .sw:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
  }

  .sw.on {
    background: var(--color-accent);
    border-color: var(--color-accent);
  }

  .thumb {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 14px;
    height: 14px;
    border-radius: 999px;
    background: var(--foreground);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
    transition: transform 180ms cubic-bezier(0.4, 0, 0.2, 1);
  }

  .sw.on .thumb {
    background: var(--color-accent-foreground);
    transform: translateX(14px);
  }
</style>
