<script lang="ts">
  import type { Snippet } from "svelte";
  import { cn } from "../../../utils.js";
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

<!-- No responsive breakpoints: this is a desktop app, the sidebar exists at
     every window width; collapsing is the offcanvas/icon mode's job. -->
<div
  data-slot="sidebar"
  data-state={state}
  data-collapsible={sidebar.open ? "" : collapsible}
  class="group peer block text-sidebar-foreground"
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
      "fixed inset-y-0 start-0 z-10 flex h-svh w-(--sidebar-width) transition-[inset-inline-start,width] duration-200 ease-linear",
      "group-data-[collapsible=offcanvas]:start-[calc(var(--sidebar-width)*-1)]",
      "group-data-[collapsible=icon]:w-(--sidebar-width-icon)",
      "border-e border-sidebar-border",
      className
    )}
  >
    <!-- Slid off the canvas, the rail's controls are hidden and not merely out
         of sight: a button at x = -41 still took Tab and a click, so the keyboard
         walked through a sidebar nobody could see. `visibility` flips at the END
         of a transition to hidden and at the START of one to visible, so the
         slide still shows both ways. The rail re-shows itself; it is how a
         pointer brings the sidebar back. -->
    <div
      data-sidebar="sidebar"
      data-slot="sidebar-inner"
      class="flex h-full w-full flex-col bg-sidebar transition-[visibility] duration-200 group-data-[collapsible=offcanvas]:invisible"
    >
      {@render children?.()}
    </div>
  </div>
</div>
