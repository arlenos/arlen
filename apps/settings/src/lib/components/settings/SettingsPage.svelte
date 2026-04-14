<script lang="ts">
  /// Wrapper for a full settings panel page.
  /// Provides the title header + max-width content container.
  import type { Snippet } from "svelte";

  let {
    title,
    description,
    children,
  }: { title: string; description?: string; children?: Snippet } = $props();
</script>

<div class="mx-auto w-full max-w-2xl px-6 py-8">
  <header class="mb-8">
    <h1 class="text-2xl font-semibold tracking-tight">{title}</h1>
    {#if description}
      <p class="mt-1 text-sm text-muted-foreground">{description}</p>
    {/if}
  </header>
  <div class="space-y-8">
    {@render children?.()}
  </div>
</div>
