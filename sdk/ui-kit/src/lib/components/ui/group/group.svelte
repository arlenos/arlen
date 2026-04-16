<script lang="ts">
  /// Settings group: a small uppercase label followed by a rounded
  /// card with divided rows. macOS System Settings pattern.
  import type { Snippet } from "svelte";

  let {
    label,
    children,
  }: { label: string; children?: Snippet } = $props();
</script>

<div class="group">
  <div class="group-label">{label}</div>
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
    padding-left: 0.25rem;
  }

  .group-card {
    border-radius: var(--radius-lg);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 10%, transparent);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    /* No overflow:hidden, so dropdown menus can escape the card. */
  }

  .group-card :global(> * + *) {
    border-top: 1px solid
      color-mix(in srgb, var(--foreground) 7%, transparent);
  }
</style>
