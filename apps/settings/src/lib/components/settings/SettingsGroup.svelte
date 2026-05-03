<script lang="ts">
  /// Section within a settings panel.
  /// Renders a small uppercase label and a card-style container for rows.
  import type { Snippet } from "svelte";

  let { label, children }: { label: string; children?: Snippet } = $props();
</script>

<section class="space-y-2">
  <h2
    class="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
  >
    {label}
  </h2>
  <div
    class="divide-y divide-border overflow-hidden rounded-card border border-border bg-card"
  >
    {@render children?.()}
  </div>
</section>
