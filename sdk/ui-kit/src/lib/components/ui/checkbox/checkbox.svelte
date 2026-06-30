<script lang="ts">
  /// The kit checkbox: a square selection box on the chip radius (so it follows
  /// the Roundness slider), filling with a check when set. Wraps the bits-ui
  /// Checkbox for the role / aria-checked / keyboard (Space) behaviour; the look
  /// is the flat house style shared with the Switch / SegmentedControl. Use it
  /// anywhere a single on/off or pick-one selection sits inline (a settings row,
  /// a list row's "default" marker), instead of hand-rolled glyphs.
  import { Checkbox as CheckboxPrimitive } from "bits-ui";
  import { Check } from "@lucide/svelte";
  import { cn } from "$lib/utils.js";

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
    "inline-flex size-4 shrink-0 items-center justify-center rounded-chip border border-border bg-input text-primary-foreground transition-[background-color,border-color,box-shadow] duration-fast ease-out outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=checked]:bg-primary data-[state=checked]:border-primary disabled:opacity-50 disabled:pointer-events-none",
    className,
  )}
>
  {#snippet children(state: { checked: boolean; indeterminate: boolean })}
    {#if state.checked}
      <Check class="size-3" strokeWidth={3} />
    {/if}
  {/snippet}
</CheckboxPrimitive.Root>
