<script lang="ts">
  /// The FM sidebar: the kit place groups (Places, Devices, and the
  /// quiet KG Projects group when the graph has any) inside the kit
  /// Sidebar shell. Navigation goes to the ACTIVE tab's controller —
  /// places are locations, not tabs.
  import { get } from "svelte/store";
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { PlacesSidebar, placeIcon } from "@arlen/ui-kit/components/browser";
  import { activeController } from "$lib/stores/tabs";
  import { placeGroups, savedSearches } from "$lib/stores/places";
  import { runSearch, searchOpen, searchQuery } from "$lib/stores/search";

  const SearchIcon = placeIcon("search");

  // The active location, live, for the place highlight.
  let activePath = $state("");
  $effect(() => {
    const c = $activeController;
    if (!c) return;
    return c.path.subscribe((p) => (activePath = p));
  });

  /// A saved search opens the bar with its query over the current
  /// location (KG quiet place #1b: queries as places).
  function pickSearch(query: string) {
    searchOpen.set(true);
    searchQuery.set(query);
    const c = get(activeController);
    if (c) void runSearch(get(c.path));
  }
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
    {#if $savedSearches.length > 0}
      <SidebarGroup class="group-data-[collapsible=icon]:hidden">
        <SidebarGroupLabel>Searches</SidebarGroupLabel>
        <SidebarMenu>
          {#each $savedSearches as s (s.id)}
            <SidebarMenuItem>
              <SidebarMenuButton
                tooltip={s.query}
                onclick={() => pickSearch(s.query)}
              >
                <SearchIcon />
                <span class="fs-label">{s.name}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          {/each}
        </SidebarMenu>
      </SidebarGroup>
    {/if}
  </SidebarContent>

  <SidebarRail />
</Sidebar>

<style>
  .fs-label {
    font-size: 0.8125rem;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
