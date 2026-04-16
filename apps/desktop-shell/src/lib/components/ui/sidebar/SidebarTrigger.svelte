<script lang="ts">
  import { cn } from "$lib/utils";
  import { PanelLeft } from "lucide-svelte";
  import { useSidebar } from "./context.svelte";

  let { class: className }: { class?: string } = $props();
  const sidebar = useSidebar();
</script>

<button
  type="button"
  data-slot="sidebar-trigger"
  data-sidebar="trigger"
  aria-label="Toggle Sidebar"
  onclick={() => sidebar.toggle()}
  class={cn(
    "inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground transition-colors",
    className
  )}
>
  <PanelLeft size={16} />
</button>
