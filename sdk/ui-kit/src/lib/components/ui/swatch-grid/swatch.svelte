<script lang="ts">
  /// One tile in a `SwatchGrid`: a preview on top (a solid colour, or any
  /// snippet), the name under it, and a check when it is the one in force.
  /// The tile is a button with `aria-pressed`, so a screen reader hears the
  /// same fact the check shows.
  ///
  /// Radius follows the nested rule (design-system.md): the tile is a card,
  /// the preview sits at zero inset and is clipped by the tile, so no second
  /// radius is written for it. A preview that draws its own inner shape uses
  /// `--swatch-inner`, which is the card radius minus the inset it chose.
  import type { Snippet } from "svelte";
  import { Check } from "@lucide/svelte";

  let {
    label,
    active = false,
    color,
    activeText,
    id,
    onclick,
    preview,
  }: {
    label: string;
    active?: boolean;
    /// A solid preview. Ignored when `preview` is given.
    color?: string;
    /// The word beside the check when this tile is in force ("Active").
    activeText?: string;
    id?: string;
    onclick?: () => void;
    preview?: Snippet;
  } = $props();
</script>

<button type="button" class="swatch" class:active aria-pressed={active} {id} {onclick}>
  <span class="swatch-preview" style={preview ? undefined : `background:${color}`} aria-hidden="true">
    {@render preview?.()}
  </span>
  <span class="swatch-foot">
    <span class="swatch-name">{label}</span>
    {#if active}
      <span class="swatch-check">
        <Check size={13} strokeWidth={2.5} aria-hidden="true" />
        {#if activeText}<span>{activeText}</span>{/if}
      </span>
    {/if}
  </span>
</button>

<style>
  .swatch {
    --swatch-inner: calc(var(--radius-card) - 0.625rem);
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    border-radius: var(--radius-card);
    background: var(--card, var(--background));
    color: var(--foreground);
    text-align: start;
    overflow: hidden;
    transition:
      border-color var(--duration-fast, 120ms) var(--ease-default, ease),
      background-color var(--duration-fast, 120ms) var(--ease-default, ease);
  }
  .swatch:hover {
    border-color: color-mix(in srgb, var(--foreground) 25%, transparent);
  }
  .swatch.active {
    border-color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .swatch:focus-visible {
    outline: 2px solid var(--ring, var(--foreground));
    outline-offset: 2px;
  }

  .swatch-preview {
    display: block;
    aspect-ratio: 16 / 7;
    width: 100%;
  }

  .swatch-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    min-height: calc(var(--height-control, 30px) + 0.25rem);
  }
  .swatch-name {
    font-size: var(--text-sm);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .swatch-check {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--color-success);
  }

  /* Dense tiles (a palette): the preview is a square and the foot is one
     small line. */
  :global(.swatch-grid.dense) .swatch-preview {
    aspect-ratio: 1;
  }
  :global(.swatch-grid.dense) .swatch-foot {
    padding: 0.25rem 0.5rem;
    min-height: 0;
  }
  :global(.swatch-grid.dense) .swatch-name {
    font-size: var(--text-2xs);
    font-weight: 400;
  }
</style>
