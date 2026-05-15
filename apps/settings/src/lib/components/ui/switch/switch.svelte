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
    /// "default" — 32×18 track, 14px thumb; "sm" — 24×14 track,
    /// 10px thumb. Documented sizing-system outlier (iOS/Android
    /// binary-toggle convention; track-width covers the 24px
    /// hit-target floor without needing height-floor compliance).
    /// See docs/architecture/sizing-system.md §5a.
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
    border-radius: var(--radius-input);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 14%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition:
      transform var(--duration-micro) var(--ease-out),
      background-color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }

  .sw:active:not(:disabled) {
    transform: scale(0.94);
  }

  /* Switch is a documented sizing-system outlier (see
     docs/architecture/sizing-system.md §5a). Binary-toggle convention
     follows iOS/Android, where the track sits below the standard
     control register because the WIDTH (32 / 24) covers the 24px
     hit-target floor — the entire component IS the click target,
     not a sub-element. */
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
    border-radius: var(--radius-input);
    background: var(--foreground);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
    transition: transform var(--duration-medium) var(--ease-out);
  }

  /* Thumb size = track height − 4 (1px border each side + 1px gap
     each side). Centred via top:1 (gap is 1px on top and bottom
     after subtracting 1px border each, which the box-sizing:
     border-box on .sw resolves automatically). */
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
  /* On-state translate = track inner width − thumb width.
     default: (32 − 2*1 border) − 14 = 16, less the left:1 offset = 14.
     sm:      (24 − 2*1) − 10 = 12, less left:1 = 10. */
  .sw.default.on .thumb {
    transform: translateX(14px);
  }
  .sw.sm.on .thumb {
    transform: translateX(10px);
  }
</style>
