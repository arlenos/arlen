<script lang="ts">
  import type { Snippet } from "svelte";
  import { cn } from "$lib/utils";

  let {
    class: className,
    children,
  }: { class?: string; children?: Snippet } = $props();
</script>

<!-- `min-w-0`: the inset is the flexible pane beside a fixed-width sidebar;
     without it, wide content (a grid, a long unbreakable line) sets the flex
     minimum and pushes the whole inset past the viewport instead of
     shrinking. -->
<main
  data-slot="sidebar-inset"
  class={cn(
    "relative flex min-h-svh min-w-0 flex-1 flex-col bg-background",
    className
  )}
>
  {@render children?.()}
</main>
