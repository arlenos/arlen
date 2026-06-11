<script lang="ts">
  /// The FM sidebar: the kit place groups (Places, Devices, and the
  /// quiet KG Projects group when the graph has any) inside the kit
  /// Sidebar shell. Navigation goes to the ACTIVE tab's controller —
  /// places are locations, not tabs.
  import {
    Sidebar,
    SidebarContent,
    SidebarHeader,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { PlacesSidebar } from "@arlen/ui-kit/components/browser";
  import { activeController } from "$lib/stores/tabs";
  import { placeGroups } from "$lib/stores/places";

  // The active location, live, for the place highlight.
  let activePath = $state("");
  $effect(() => {
    const c = $activeController;
    if (!c) return;
    return c.path.subscribe((p) => (activePath = p));
  });
</script>

<Sidebar collapsible="icon">
  <SidebarHeader class="h-10 flex-row items-center py-0">
    <span
      class="px-2 text-[0.6875rem] font-semibold uppercase tracking-[0.1em] text-sidebar-foreground/55 group-data-[collapsible=icon]:hidden"
    >
      Files
    </span>
  </SidebarHeader>

  <SidebarContent>
    <PlacesSidebar
      groups={$placeGroups}
      {activePath}
      onnavigate={(place) => $activeController?.navigate(place.path)}
    />
  </SidebarContent>

  <SidebarRail />
</Sidebar>
