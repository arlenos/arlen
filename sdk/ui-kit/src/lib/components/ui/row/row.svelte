<script lang="ts">
  /// A single row inside a Group card. Label on the left, control on the
  /// right, with an optional inline preview between them, and an optional
  /// full-width `below` area for wide controls (a list, a chip editor) that
  /// do not fit the right-aligned control slot.
  import type { Snippet } from "svelte";

  let {
    label,
    id: rowId,
    description,
    leading,
    control,
    preview,
    below,
  }: {
    label: string;
    /// Optional anchor id for deep-link scroll-to-setting.
    id?: string;
    description?: string;
    /// Optional leading visual before the label (a logo, badge, or rank), for
    /// rows that need an icon column the bare label/control layout lacks.
    leading?: Snippet;
    control?: Snippet;
    preview?: Snippet;
    /// Optional full-width content rendered under the label/control line.
    below?: Snippet;
  } = $props();
</script>

<div class="row" id={rowId}>
  <div class="row-main">
    {#if leading}
      <div class="leading">{@render leading()}</div>
    {/if}
    <div class="label">
      <div class="label-title">{label}</div>
      {#if description}
        <div class="label-desc">{description}</div>
      {/if}
    </div>
    {#if preview}
      <div class="preview">
        {@render preview()}
      </div>
    {/if}
    <div class="control">
      {@render control?.()}
    </div>
  </div>
  {#if below}
    <div class="row-below">
      {@render below()}
    </div>
  {/if}
</div>

<style>
  .row {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    /* Top/bottom uses the row spacing token; horizontal stays fixed at 1rem
       to give the label a reliable left edge regardless of token overrides. */
    padding: var(--space-row, 0.75rem) 1rem;
  }

  .row-main {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    /* Preserve the row rhythm: the main line keeps the standard row
       height; the .row padding is excluded here so a `below` block adds
       under it rather than inflating the line. */
    min-height: calc(var(--height-row, 40px) - 1.5rem);
  }

  .leading {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }

  .label {
    flex: 1;
    min-width: 0;
  }

  /* Title truncates; description wraps to multiple lines if it
     can't fit on one. This keeps the row height stable per
     row-rhythm spec while still allowing prose-style hints
     below the title. */
  .label-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .label-desc {
    font-size: var(--text-2xs);
    line-height: 1.3;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    margin-top: 0.0625rem;
  }

  .preview,
  .control {
    flex-shrink: 0;
  }

  /* Full-width content under the row line. */
  .row-below {
    min-width: 0;
  }
</style>
