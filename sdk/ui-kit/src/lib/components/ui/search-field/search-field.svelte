<script lang="ts">
  /// The one search/filter field. Every surface that searches or filters uses
  /// this instead of hand-rolling icon + input: same height, same icon
  /// placement, same tone, RTL-safe via logical properties. The placeholder
  /// stays the caller's - it names what is searched, the field only looks the
  /// same everywhere. `--search-radius` lets a concentric context (the settings
  /// sidebar corner) derive its radius without forking the component.
  import { Search } from "@lucide/svelte";
  import { cn } from "../../../utils.js";
  import type { HTMLInputAttributes } from "svelte/elements";

  /// Named rather than written inline at the destructure, which is not a style
  /// preference. Written inline, the identical intersection resolved `size` to
  /// `never`, so `size = "control"` below failed with "Type 'string' is not
  /// assignable to type 'never'" - the field's own `size` (28px row or 36px hero)
  /// collides with the input element's numeric `size`, which the `Omit` is there
  /// to remove. Moving the same types into a named alias resolves it. The types
  /// are unchanged, so the cause is in how the annotation is processed rather
  /// than in the types; keep it named.
  type Props = Omit<HTMLInputAttributes, "size"> & {
    ref?: HTMLInputElement | null;
    value?: string;
    /// "control" is the 28px row height; "prominent" the 36px hero field.
    size?: "control" | "prominent";
  };

  let {
    ref = $bindable(null),
    value = $bindable(""),
    placeholder,
    size = "control",
    class: className,
    ...rest
  }: Props = $props();
</script>

<div class={cn("sf", size === "prominent" && "sf-prominent", className)}>
  <Search size={size === "prominent" ? 14 : 13} strokeWidth={2} class="sf-icon" aria-hidden="true" />
  <input bind:this={ref} bind:value type="search" data-slot="input" class="sf-input" {placeholder} {...rest} />
</div>

<style>
  .sf {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    min-width: 0;
  }
  .sf :global(.sf-icon) {
    position: absolute;
    inset-inline-start: 0.55rem;
    pointer-events: none;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .sf-input {
    width: 100%;
    height: var(--height-control, 28px);
    padding-block: 0.25rem;
    padding-inline: 1.65rem 0.625rem;
    border: 1px solid var(--border);
    border-radius: var(--search-radius, var(--radius-input));
    background: var(--input, transparent);
    font-family: inherit;
    font-size: var(--text-sm);
    color: var(--foreground);
    transition:
      background-color var(--duration-fast, 150ms) var(--ease-out, ease),
      border-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .sf-prominent .sf-input {
    height: var(--height-control-prominent, 36px);
    padding-inline-start: 1.85rem;
  }
  .sf-prominent :global(.sf-icon) {
    inset-inline-start: 0.65rem;
  }
  .sf-input::placeholder {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .sf-input:focus-visible {
    outline: none;
    border-color: var(--ring, var(--border));
  }
  /* The native clear button of type=search is styled away; Escape clears. */
  .sf-input::-webkit-search-cancel-button {
    display: none;
  }
</style>
