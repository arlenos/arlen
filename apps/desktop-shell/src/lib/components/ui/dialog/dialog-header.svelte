<script lang="ts">
  /// Vertical title + description block at the top of a dialog.

  import { cn } from "$lib/utils.js";
  import type { Snippet } from "svelte";

  let {
    class: className,
    children,
  }: { class?: string; children?: Snippet } = $props();
</script>

<div
  data-slot="dialog-header"
  class={cn("mb-3 flex flex-col gap-1", className)}
>
  {@render children?.()}
</div>
