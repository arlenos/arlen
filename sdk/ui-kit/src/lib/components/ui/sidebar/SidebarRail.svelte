<script lang="ts">
  import { cn } from "$lib/utils";
  import { useSidebar } from "./context.svelte";

  let { class: className }: { class?: string } = $props();
  const sidebar = useSidebar();
</script>

<button
  type="button"
  data-slot="sidebar-rail"
  data-sidebar="rail"
  aria-label="Toggle Sidebar"
  tabindex={-1}
  onclick={() => sidebar.toggle()}
  title="Toggle Sidebar"
  class={cn(
    "absolute inset-y-0 z-20 hidden w-4 -translate-x-1/2 transition-all ease-linear after:absolute after:inset-y-0 after:left-1/2 after:w-[2px] hover:after:bg-sidebar-border sm:flex",
    "in-data-[side=left]:cursor-w-resize in-data-[side=right]:cursor-e-resize",
    "[[data-side=left]_&]:right-0 [[data-side=right]_&]:left-0",
    "hover:group-data-[collapsible=offcanvas]:bg-sidebar group-data-[collapsible=offcanvas]:translate-x-0 group-data-[collapsible=offcanvas]:after:left-full",
    "[[data-side=left][data-collapsible=offcanvas]_&]:-right-2",
    "[[data-side=right][data-collapsible=offcanvas]_&]:-left-2",
    className
  )}
></button>
