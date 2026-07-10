<script lang="ts">
  /// The FM sidebar: the kit place groups (Places, Devices, and the
  /// quiet KG Projects group when the graph has any) inside the kit
  /// Sidebar shell. Navigation goes to the ACTIVE tab's controller —
  /// places are locations, not tabs.
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
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
  import { Trash2, Clock, SlidersHorizontal } from "lucide-svelte";
  import { activeController } from "$lib/stores/tabs";
  import { placeGroups, removePlace, navigatePlace, savedSearches } from "$lib/stores/places";
  import { runSearch, searchOpen, searchQuery } from "$lib/stores/search";
  import {
    savedFolders,
    applySmartFolder,
    facetOpen,
    type SmartFolder,
  } from "$lib/stores/facets";
  import { t } from "$lib/i18n/messages";

  const SearchIcon = placeIcon("search");

  /// Recent + Trash are navigation locations (not overlays): navigating the
  /// active controller to their virtual key lists them in the normal file view.
  async function goLocation(location: string) {
    const c = get(activeController);
    if (!c) return;
    // Conclusive one-run bisect for the virtual-location bug (Trash showed home
    // on metal, no DevTools): log the target, the path before, and the path after
    // navigate resolves. If `after` is not the location, navigate returned early
    // (guard / stale build) - a frontend break; if `after` IS the location but
    // the view still shows home, the break is downstream (adapter/render), paired
    // with the `fmAdapter.list` + backend `files_list_location` logs.
    const before = get(c.path);
    await c.navigate(location);
    void invoke("frontend_log", {
      level: "info",
      msg: `goLocation: target=${location} before=${before} after=${get(c.path)}`,
    });
  }

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

  /// A Smart Folder re-applies its saved facets, reveals the filter bar so the
  /// active chips show, and navigates the listing to the faceted result.
  function pickFolder(folder: SmartFolder) {
    applySmartFolder(folder);
    facetOpen.set(true);
    void get(activeController)?.navigate(folder.location);
  }
</script>

<Sidebar collapsible="icon">
  <SidebarHeader class="h-10 flex-row items-center py-0">
    <span
      class="px-2 text-[0.6875rem] font-semibold uppercase tracking-[0.1em] text-sidebar-foreground/55 group-data-[collapsible=icon]:hidden"
    >
      {$t("f.sidebar.files")}
    </span>
  </SidebarHeader>

  <SidebarContent class="fm-sidebar-scroll">
    <PlacesSidebar
      groups={$placeGroups}
      {activePath}
      onnavigate={(place) => {
        const c = $activeController;
        if (c) void navigatePlace(place, (p) => c.navigate(p));
      }}
      onremove={(place) => removePlace(place)}
    />
    {#if $savedFolders.length > 0}
      <SidebarGroup class="group-data-[collapsible=icon]:hidden">
        <SidebarGroupLabel>{$t("f.sidebar.smartFolders")}</SidebarGroupLabel>
        <SidebarMenu>
          {#each $savedFolders as f (f.id)}
            <SidebarMenuItem>
              <SidebarMenuButton tooltip={f.name} onclick={() => pickFolder(f)}>
                <SlidersHorizontal />
                <span class="fs-label">{f.name}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          {/each}
        </SidebarMenu>
      </SidebarGroup>
    {/if}
    {#if $savedSearches.length > 0}
      <SidebarGroup class="group-data-[collapsible=icon]:hidden">
        <SidebarGroupLabel>{$t("f.sidebar.searches")}</SidebarGroupLabel>
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

    <!-- Recent + Trash sit at the foot of the sidebar; both are navigation
         locations (the KG recent files / the trash), highlighted when active. -->
    <SidebarGroup class="group-data-[collapsible=icon]:hidden">
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton
            tooltip={$t("f.sidebar.recent")}
            isActive={activePath === "recent"}
            onclick={() => goLocation("recent")}
          >
            <Clock />
            <span class="fs-label">{$t("f.sidebar.recent")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
        <SidebarMenuItem>
          <SidebarMenuButton
            tooltip={$t("f.sidebar.trash")}
            isActive={activePath === "trash"}
            onclick={() => goLocation("trash")}
          >
            <Trash2 />
            <span class="fs-label">{$t("f.sidebar.trash")}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarGroup>
  </SidebarContent>

  <SidebarRail />
</Sidebar>

<style>
  /* A sticky fade at the scroll edge says "more below" instead of a
     hard mid-row cut. */
  :global(.fm-sidebar-scroll)::after {
    content: "";
    position: sticky;
    bottom: 0;
    display: block;
    height: 16px;
    margin-top: -16px;
    flex-shrink: 0;
    background: linear-gradient(to bottom, transparent, var(--sidebar));
    pointer-events: none;
  }

  .fs-label {
    font-size: 0.8125rem;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
