<script lang="ts">
  import type { Snippet } from "svelte";
  import { cn } from "$lib/utils";

  let {
    class: className,
    children,
  }: { class?: string; children?: Snippet } = $props();
</script>

<li
  data-slot="sidebar-menu-item"
  data-sidebar="menu-item"
  class={cn("group/menu-item relative", className)}
>
  {@render children?.()}
</li>
