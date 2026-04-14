<script lang="ts">
  import type { Snippet } from "svelte";
  import { cn } from "$lib/utils";

  let {
    class: className,
    children,
  }: { class?: string; children?: Snippet } = $props();
</script>

<main
  data-slot="sidebar-inset"
  class={cn(
    "relative flex min-h-svh flex-1 flex-col bg-background",
    className
  )}
>
  {@render children?.()}
</main>
