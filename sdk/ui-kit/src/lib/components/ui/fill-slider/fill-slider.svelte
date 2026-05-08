<script lang="ts">
  /// Lunaris fill-bar slider.
  ///
  /// 24px-tall rounded rectangle. The accent-tinted fill anchored at
  /// the left represents the value; there is no thumb circle. Reads
  /// as a flat data bar, in the same visual register as `Switch`
  /// (matching radius-input, fg-shell-tinted track, hover treatment).
  ///
  /// Pointer interaction is a hidden full-overlay `<input type="range">`
  /// — keeps native drag, click-to-jump, keyboard arrows (Left/Right,
  /// Home/End, PageUp/PageDown), and accessibility for free.
  ///
  /// Stateless: caller passes `value`, receives changes via `oninput`.
  /// The component does not own state — that lets backends (logind
  /// brightness, wpctl volume, fs-watched config files) stay
  /// authoritative.

  let {
    value,
    min = 0,
    max = 100,
    step = 1,
    size = "default",
    disabled = false,
    ariaLabel,
    oninput,
    onfocus,
    onblur,
  }: {
    value: number;
    min?: number;
    max?: number;
    step?: number;
    /// `"default"` — 24px, for QuickSettings tiles and Settings rows
    /// where the slider has its own breathing room. `"sm"` — 12px,
    /// for dense popover rows that sit alongside small icons / value
    /// text where a chunky bar would dominate the layout. Mirrors
    /// `Switch`'s `default`/`sm` size pattern.
    size?: "default" | "sm";
    disabled?: boolean;
    ariaLabel?: string;
    /// Fired on every range change. Caller decides whether to debounce
    /// hardware writes (typical pattern: setTimeout 32ms for ~30Hz).
    oninput?: (value: number) => void;
    onfocus?: () => void;
    onblur?: () => void;
  } = $props();

  const percent = $derived(
    Math.max(0, Math.min(100, ((value - min) / (max - min)) * 100)),
  );
</script>

<div class="ln-fill-slider {size}" class:disabled style="--percent: {percent}%">
  <div class="ln-fill-slider-track">
    <div class="ln-fill-slider-fill"></div>
  </div>
  <input
    type="range"
    {min}
    {max}
    {step}
    {value}
    {disabled}
    aria-label={ariaLabel}
    oninput={(e) => oninput?.(parseFloat(e.currentTarget.value))}
    onfocus={() => onfocus?.()}
    onblur={() => onblur?.()}
  />
</div>

<style>
  .ln-fill-slider {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    cursor: pointer;
  }
  /* Wrapper height matches the standard inline-control register
     (28 / 24) so the click area aligns vertically with sibling
     Input / Select / TimeInput / etc. in a settings row. The
     VISIBLE track inside is much thinner (8 / 6) — slider's visual
     mass should match a 1px-bordered input's empty interior, not
     a solid filled rectangle. Adwaita uses 10px track, Apple ~4-6,
     so 8 default sits in the middle. See
     docs/architecture/sizing-system.md §3-§5 for the visual-mass
     rationale. */
  .ln-fill-slider.default {
    height: var(--height-control, 28px);
  }
  .ln-fill-slider.sm {
    height: var(--height-control-compact, 24px);
  }
  .ln-fill-slider.disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Track is the VISIBLE bar — not the wrapper. It's centred
     vertically inside the wrapper, with the slim outer wrapper
     providing the larger hit-target. */
  .ln-fill-slider-track {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    transform: translateY(-50%);
    border-radius: var(--radius-input);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 12%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    overflow: hidden;
    transition:
      border-color var(--duration-fast, 150ms) var(--ease-out, ease),
      background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }

  /* Visible-bar sizing: mid-register between "thin Adwaita track"
     (8-10px, hard to aim at on touchpads) and "full filled
     rectangle" (24-28px, dominates the row). 18 / 14 lands at ~65%
     of wrapper height — clearly a bar with comfortable hit-feel,
     visually paired with the 18px Switch (both specialised). */
  .ln-fill-slider.default .ln-fill-slider-track {
    height: 18px;
  }
  .ln-fill-slider.sm .ln-fill-slider-track {
    height: 14px;
    border-color: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .ln-fill-slider:hover:not(.disabled) .ln-fill-slider-track {
    border-color: color-mix(in srgb, var(--foreground) 22%, transparent);
  }

  .ln-fill-slider:focus-within .ln-fill-slider-track {
    border-color: var(--color-accent, var(--primary));
  }

  /* Fill takes the full track height. Lower-saturation accent
     (40%) keeps visual weight from dominating the row even though
     the bar is comfortably tall. */
  .ln-fill-slider-fill {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: var(--percent);
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 40%, transparent);
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }

  input[type="range"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0;
    opacity: 0;
    cursor: inherit;
    appearance: none;
    -webkit-appearance: none;
  }
  input[type="range"]:focus-visible {
    outline: none;
  }
</style>
