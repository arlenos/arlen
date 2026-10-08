<script lang="ts">
  import { kt } from "../../../i18n/messages.kit";
  import { cn } from "../../../utils.js";
  import { useSidebar } from "./context.svelte";

  let { class: className }: { class?: string } = $props();
  const sidebar = useSidebar();
</script>

<!-- OUT OF THE TAB ORDER ON PURPOSE, and the reason belongs here because the next
     reader will see a labelled button with `tabindex={-1}` and want to fix it. The
     rail is the 4px strip along the sidebar's edge: a pointer convenience that
     duplicates `SidebarTrigger`, which every app using it has in its header
     (checked: settings, files, pdf, knowledge, terminal, mail, calendar, meetings).
     A 4px invisible strip in the tab order is a stop with nothing to look at,
     landing on the same action one Tab away. It stays reachable by keyboard
     through the trigger; the day an app ships the rail WITHOUT one, this is the
     line to revisit.

     No tooltip either: the strip is found by its resize cursor, and a native
     title bubble over a drag edge is noise (design-system §6.4, rule 3). The
     accessible name stays. -->
<button
  type="button"
  data-slot="sidebar-rail"
  data-sidebar="rail"
  aria-label={$kt("k.sidebar.toggle")}
  tabindex={-1}
  onclick={() => sidebar.toggle()}
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
