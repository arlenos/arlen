<script lang="ts">
  /// Lunaris Switch — unified toggle for Shell + Settings.
  /// Follows the corner radius system and uses the accent colour
  /// for the on-state. Replaces the shadcn-svelte bits-ui switch
  /// which had hardcoded colours and rounded-full.

  let {
    value = $bindable(false),
    ariaLabel,
    disabled = false,
    size = "default",
    onchange,
    class: className,
  }: {
    value?: boolean;
    ariaLabel?: string;
    disabled?: boolean;
    /// "default" = 32x18px, "sm" = 24x14px.
    size?: "default" | "sm";
    onchange?: (value: boolean) => void;
    class?: string;
  } = $props();

  function toggle() {
    if (disabled) return;
    value = !value;
    onchange?.(value);
  }
</script>

<button
  type="button"
  role="switch"
  aria-checked={value}
  aria-label={ariaLabel}
  {disabled}
  class="sw {size} {className ?? ''}"
  class:on={value}
  onclick={toggle}
>
  <span class="thumb"></span>
</button>

<style>
  .sw {
    position: relative;
    border-radius: var(--radius-md);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 14%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition:
      background-color 150ms ease,
      border-color 150ms ease;
  }

  .sw.default {
    width: 32px;
    height: 18px;
  }
  .sw.sm {
    width: 24px;
    height: 14px;
  }

  .sw:hover:not(:disabled):not(.on) {
    border-color: color-mix(in srgb, var(--foreground) 25%, transparent);
  }

  .sw.on {
    background: var(--color-accent, var(--primary));
    border-color: var(--color-accent, var(--primary));
  }

  .sw:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .thumb {
    position: absolute;
    top: 1px;
    left: 1px;
    border-radius: var(--radius-md);
    background: var(--foreground);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
    transition: transform 180ms cubic-bezier(0.4, 0, 0.2, 1);
  }

  .sw.default .thumb {
    width: 14px;
    height: 14px;
  }
  .sw.sm .thumb {
    width: 10px;
    height: 10px;
  }

  .sw.on .thumb {
    background: var(--color-accent-foreground, var(--primary-foreground, #fff));
  }
  .sw.default.on .thumb {
    transform: translateX(14px);
  }
  .sw.sm.on .thumb {
    transform: translateX(10px);
  }
</style>
