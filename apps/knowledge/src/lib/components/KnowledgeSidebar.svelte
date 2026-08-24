<script lang="ts">
  /// The Knowledge places sidebar on the files-canon kit primitives: icon
  /// collapse, the caps app label in the header, group labels, a rail. The
  /// explore places over the graph plus the rows that link out to
  /// Settings/Privacy (knowledge-app.md decision 6 - a surface that owns a
  /// capability is not re-hosted here).
  import { Clock, FolderGit2, Search, Library, Package, ShieldCheck, ChevronRight } from "lucide-svelte";
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { t } from "$lib/i18n/messages";

  let {
    activeLocation,
    onnavigate,
    onsettings,
  }: {
    activeLocation: string;
    onnavigate: (location: string) => void;
    onsettings: () => void;
  } = $props();

  // The explore places with their icons, in sidebar order (§2). The label/empty
  // presentation lives in `locations.ts`; the icon pairing lives here.
  const PLACES = [
    { id: "timeline", labelKey: "k.place.timeline", icon: Clock },
    { id: "projects", labelKey: "k.place.projects", icon: FolderGit2 },
    { id: "searches", labelKey: "k.place.searches", icon: Search },
    { id: "library", labelKey: "k.place.library", icon: Library },
  ];

  // The rows that leave for Settings rather than being re-hosted here (decision 6).
  const LINKOUTS = [
    { id: "capabilities", labelKey: "k.place.capabilities", icon: ShieldCheck },
    { id: "capsules", labelKey: "k.place.capsules", icon: Package },
  ];

  function isActive(id: string): boolean {
    const scheme = activeLocation.split(":")[0] ?? activeLocation;
    if (id === activeLocation) return true;
    if (id === "searches" && scheme === "search") return true;
    if (id === "projects" && scheme === "project") return true;
    return false;
  }
</script>

<Sidebar collapsible="icon">
  <SidebarContent>
    <!-- pt-1: the group label's centre sits on the h-10 header-bar line.
         Collapsed the label vanishes, so the first icon row takes the edge
         with the same 6px gap the header-bar icons keep; the box stays the
         rail's uniform size. -->
    <SidebarGroup class="pt-1 group-data-[collapsible=icon]:pt-1.5">
      <SidebarGroupLabel>{$t("k.section.explore")}</SidebarGroupLabel>
      <SidebarMenu>
        {#each PLACES as p (p.id)}
          {@const Icon = p.icon}
          <SidebarMenuItem>
            <!-- Addressable by place (`data-place`) so headless shots reach
                 Projects or Library without :nth-of-type. -->
            <SidebarMenuButton
              data-place={p.id}
              isActive={isActive(p.id)}
              onclick={() => onnavigate(p.id)}
            >
              <Icon strokeWidth={1.75} />
              <span>{$t(p.labelKey)}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
    </SidebarGroup>

    <SidebarGroup class="pt-0">
      <SidebarGroupLabel>{$t("k.section.authority")}</SidebarGroupLabel>
      <SidebarMenu>
        {#each LINKOUTS as l (l.id)}
          {@const Icon = l.icon}
          <SidebarMenuItem>
            <SidebarMenuButton onclick={onsettings}>
              <Icon strokeWidth={1.75} />
              <span>{$t(l.labelKey)}</span>
              <ChevronRight class="ms-auto opacity-60" strokeWidth={2} />
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
      <span class="px-2 pt-0.5 text-[length:var(--text-2xs)] text-sidebar-foreground/50 group-data-[collapsible=icon]:hidden">
        {$t("k.caps.opens")}
      </span>
    </SidebarGroup>
  </SidebarContent>
  <SidebarRail />
</Sidebar>
