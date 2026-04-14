<script lang="ts">
  import type { Snippet } from "svelte";
  import { cn } from "$lib/utils";
  import { useSidebar } from "./context.svelte";

  let {
    class: className,
    collapsible = "offcanvas",
    children,
  }: {
    class?: string;
    collapsible?: "offcanvas" | "icon" | "none";
    children?: Snippet;
  } = $props();

  const sidebar = useSidebar();
  const state = $derived(sidebar.open ? "expanded" : "collapsed");
</script>

<div
  data-slot="sidebar"
  data-state={state}
  data-collapsible={sidebar.open ? "" : collapsible}
  class="group peer hidden text-sidebar-foreground md:block"
>
  <!-- Spacer that matches the sidebar width, keeps inset layout stable. -->
  <div
    data-slot="sidebar-gap"
    class={cn(
      "relative w-(--sidebar-width) bg-transparent transition-[width] duration-200 ease-linear",
      "group-data-[collapsible=offcanvas]:w-0",
      "group-data-[collapsible=icon]:w-(--sidebar-width-icon)"
    )}
  ></div>
  <!-- Fixed sidebar container. -->
  <div
    data-slot="sidebar-container"
    class={cn(
      "fixed inset-y-0 left-0 z-10 hidden h-svh w-(--sidebar-width) transition-[left,width] duration-200 ease-linear md:flex",
      "group-data-[collapsible=offcanvas]:left-[calc(var(--sidebar-width)*-1)]",
      "group-data-[collapsible=icon]:w-(--sidebar-width-icon)",
      "border-r border-sidebar-border",
      className
    )}
  >
    <div
      data-sidebar="sidebar"
      data-slot="sidebar-inner"
      class="flex h-full w-full flex-col bg-sidebar"
    >
      {@render children?.()}
    </div>
  </div>
</div>
