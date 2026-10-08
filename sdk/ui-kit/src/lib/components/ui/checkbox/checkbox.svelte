<script lang="ts">
  /// The kit checkbox: a square selection box on the chip radius (so it follows
  /// the Roundness slider), filling with a check when set. Wraps the bits-ui
  /// Checkbox for the role / aria-checked / keyboard (Space) behaviour; the look
  /// is the flat house style shared with the Switch / SegmentedControl. Use it
  /// anywhere a single on/off or pick-one selection sits inline (a settings row,
  /// a list row's "default" marker), instead of hand-rolled glyphs.
  ///
  /// Motion (design-system §6b): the act is ticking, so the tick draws itself,
  /// short stroke first, the way a hand writes it, and undraws when cleared.
  /// The box fills with it. Both run on `--duration-normal`, which the theme
  /// zeroes under reduce motion, so the tick then simply appears.
  import { Checkbox as CheckboxPrimitive } from "bits-ui";
  import { cn } from "../../../utils.js";

  let {
    checked = $bindable(false),
    disabled = false,
    ariaLabel,
    id,
    onchange,
    class: className,
  }: {
    checked?: boolean;
    disabled?: boolean;
    ariaLabel?: string;
    id?: string;
    onchange?: (checked: boolean) => void;
    class?: string;
  } = $props();
</script>

<CheckboxPrimitive.Root
  {id}
  bind:checked
  {disabled}
  aria-label={ariaLabel}
  onCheckedChange={(v) => onchange?.(v === true)}
  class={cn(
    "inline-flex size-4 shrink-0 items-center justify-center rounded-chip border border-border bg-input text-primary-foreground kit-checkbox outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=checked]:bg-primary data-[state=checked]:border-primary disabled:opacity-50 disabled:pointer-events-none",
    className,
  )}
>
  {#snippet children(state: { checked: boolean; indeterminate: boolean })}
    <!-- Lucide's check, drawn from the short stroke to the long one. -->
    <svg class="tick" class:on={state.checked} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d="M4 12l5 5L20 6" pathLength="1" />
    </svg>
  {/snippet}
</CheckboxPrimitive.Root>

<style>
  :global(.kit-checkbox) {
    transition:
      background-color var(--duration-normal) var(--ease-out),
      border-color var(--duration-normal) var(--ease-out),
      box-shadow var(--duration-normal) var(--ease-out);
  }
  .tick {
    width: 0.75rem;
    height: 0.75rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 3;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
    /* An undrawn stroke still leaves its round cap as a dot, so the cleared
       tick also hides, once the undraw has finished. */
    opacity: 0;
    transition:
      stroke-dashoffset var(--duration-normal) var(--ease-out),
      opacity 0s linear var(--duration-normal);
  }
  .tick.on {
    stroke-dashoffset: 0;
    opacity: 1;
    transition:
      stroke-dashoffset var(--duration-normal) var(--ease-out),
      opacity 0s;
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.kit-checkbox),
    .tick {
      transition: none;
    }
  }
</style>
