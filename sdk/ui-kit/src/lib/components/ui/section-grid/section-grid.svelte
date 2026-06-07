<script lang="ts">
  /// The responsive section grid: the canonical settings/app body layout.
  /// One column when narrow (readable), two columns when wide (cards reflow to
  /// use the width), capped so it never sprawls into a thin many-column wall.
  /// Holds `Group` (section) cards. A child with class `span-full` spans the
  /// whole width (for wide content: an activity log, a table, an editor).
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
    /* `min(100%, 28rem)` keeps each card readable (~448px) while collapsing to
       a single column below that width instead of overflowing. `auto-fit`
       yields two columns once the container can hold them. */
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 28rem), 1fr));
    /* Cap at two readable columns + gap so wide/ultrawide windows do not
       stretch into a thin wall; raise this token to allow a third column. */
    max-width: 64rem;
    width: 100%;
    margin-inline: auto;
  }

  /* Full-width escape hatch: wrap a section in `class="span-full"` (or set it
     on the section) to span every column. Use sparingly. */
  .section-grid :global(.span-full) {
    grid-column: 1 / -1;
  }
</style>
