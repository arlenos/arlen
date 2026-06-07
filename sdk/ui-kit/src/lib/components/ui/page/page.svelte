<script lang="ts">
  /// Canonical settings/app page wrapper: padded content region with an
  /// optional standard header (title + description). It does NOT own a scroll
  /// container — the app shell's content region already scrolls; Page only
  /// provides the page padding and header. Width + reflow are the job of
  /// `SectionGrid`, which Page centres by sharing the same max-width on its
  /// header. See `docs/architecture/design-system.md` §5.
  import type { Snippet } from "svelte";

  let {
    title,
    description,
    children,
  }: {
    title?: string;
    description?: string;
    children?: Snippet;
  } = $props();
</script>

<div class="page">
  {#if title}
    <header class="page-header">
      <h1 class="page-title">{title}</h1>
      {#if description}
        <p class="page-desc">{description}</p>
      {/if}
    </header>
  {/if}
  {@render children?.()}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    /* Use the spacing tokens so apps can override in :root without
       hunting for literal values across components. */
    gap: var(--space-section, 1.5rem);
    padding: var(--space-page, 1.5rem);
  }

  /* The header shares the grid's max-width + centring so the title lines up
     with the left edge of the section cards. */
  .page-header {
    width: 100%;
    max-width: 64rem;
    margin-inline: auto;
  }

  .page-title {
    font-size: 1.25rem;
    font-weight: 600;
    line-height: 1.2;
    color: var(--foreground);
  }

  .page-desc {
    margin-top: 0.25rem;
    font-size: 0.8125rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
