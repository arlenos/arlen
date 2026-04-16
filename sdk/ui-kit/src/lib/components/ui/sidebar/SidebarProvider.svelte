<script lang="ts">
  import type { Snippet } from "svelte";
  import { untrack } from "svelte";
  import { cn } from "$lib/utils";
  import {
    SIDEBAR_WIDTH,
    SIDEBAR_WIDTH_ICON,
    SidebarState,
    setSidebarContext,
  } from "./context.svelte";

  let {
    class: className,
    defaultOpen = true,
    children,
  }: {
    class?: string;
    defaultOpen?: boolean;
    children?: Snippet;
  } = $props();

  const state = untrack(() => new SidebarState(defaultOpen));
  setSidebarContext(state);
</script>

<div
  data-slot="sidebar-wrapper"
  class={cn(
    "group/sidebar-wrapper flex min-h-screen w-full",
    className
  )}
  style="--sidebar-width: {SIDEBAR_WIDTH}; --sidebar-width-icon: {SIDEBAR_WIDTH_ICON};"
>
  {@render children?.()}
</div>
