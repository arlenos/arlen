<script lang="ts">
  /// Settings group: a small uppercase label followed by a rounded
  /// card with divided rows. macOS System Settings pattern. The label is
  /// omitted when the page context already names the card (e.g. a feed card
  /// directly under a page title of the same name).
  import type { Snippet } from "svelte";

  let {
    label,
    children,
    class: className,
  }: { label?: string; children?: Snippet; class?: string } = $props();
</script>

<!-- `class` is forwarded to the root so a section can opt into the SectionGrid
     `span-full` escape hatch via `<Group class="span-full">`. -->
<div class="group {className ?? ''}">
  {#if label}
    <div class="group-label">{label}</div>
  {/if}
  <div class="group-card">
    {@render children?.()}
  </div>
</div>

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .group-label {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    padding-inline-start: 0.25rem;
  }

  .group-card {
    border-radius: var(--radius-card);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 10%, transparent);
    /* The card surface per theme: dark layers by lightness (the card is lighter
       than the field, --shadow-card is none); light lifts a white card off the
       grey field with --shadow-card. Falls back to the old foreground tint where
       no theme tokens are present. */
    background: var(--card, color-mix(in srgb, var(--foreground) 3%, transparent));
    box-shadow: var(--shadow-card, none);
    /* No overflow:hidden, so dropdown menus can escape the card. */
    /* Expose the card radius for concentric inset children (the rows are
       flat full-bleed dividers, but an inset rounded child reads these). */
    --container-radius: var(--radius-card);
    --container-inset: var(--space-row, 0.75rem);
  }

  .group-card :global(> * + *) {
    border-top: 1px solid
      color-mix(in srgb, var(--foreground) 7%, transparent);
  }
</style>
