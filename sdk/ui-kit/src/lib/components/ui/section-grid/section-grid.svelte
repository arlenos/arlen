<script lang="ts">
  /// The section body layout: the canonical settings/app body. A single column
  /// of full-width section cards stacked top to bottom, capped at a readable
  /// width and centred, so reading order is predictable and nothing sprawls.
  /// Holds `Group` (section) cards. `span-full` is now a no-op (every child is
  /// already full-width); it stays accepted so existing markup is untouched.
  /// See `docs/architecture/design-system.md` §5.
  import type { Snippet } from "svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<div class="section-grid">
  {@render children?.()}
</div>

<style>
  .section-grid {
    display: grid;
    /* Use the shared spacing token; the fallback matches the design-system
       spec value so this works even before the token is set in :root. */
    gap: var(--space-section, 1.5rem);
    /* One column: sections stack top to bottom, each full-width, so reading
       order is predictable (no reflowed masonry). */
    grid-template-columns: 1fr;
    /* Cap at a readable single-column width so a control stays close to its
       label and lines do not run long; a token, tune here to taste. */
    max-width: var(--width-section-body, 46rem);
    width: 100%;
    margin-inline: auto;
  }

  /* Full-width escape hatch: wrap a section in `class="span-full"` (or set it
     on the section) to span every column. Use sparingly. */
  .section-grid :global(.span-full) {
    grid-column: 1 / -1;
  }

  /* Grid items default to `min-width: auto`, so one unbreakable line (a long
     path in a log row) would silently widen its track past the grid's own
     max-width and push the page into horizontal overflow. Cap the minimum so
     content truncates inside its card instead. */
  .section-grid > :global(*) {
    min-width: 0;
  }
</style>
