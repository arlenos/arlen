<script lang="ts">
  /// The store's places sidebar on the files-canon kit primitives: icon
  /// collapse, the caps app label in the header, a rail. Browse, Installed,
  /// Updates - the three things the store is for. The update count stays a
  /// quiet number, never a red dot (update-flow-plan.md U-5).
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { Compass, Package, ArrowDownToLine } from "lucide-svelte";
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
  import { updateCount, loadUpdates } from "$lib/stores/updates";

  onMount(loadUpdates);

  const PLACES = [
    { href: "/", labelKey: "st.rail.browse", icon: Compass },
    { href: "/installed", labelKey: "st.rail.installed", icon: Package },
    { href: "/updates", labelKey: "st.rail.updates", icon: ArrowDownToLine },
  ];
  const current = $derived($page.url.pathname);
</script>

<Sidebar collapsible="icon">
  <SidebarContent>
    <!-- pt-1: the group label's centre sits on the h-10 header-bar line.
         Collapsed the label vanishes, so the first icon row takes the edge
         with the same 6px gap the header-bar icons keep; the box stays the
         rail's uniform size. -->
    <SidebarGroup class="pt-1 group-data-[collapsible=icon]:pt-1.5">
      <SidebarGroupLabel>{$t("st.section.places")}</SidebarGroupLabel>
      <SidebarMenu>
        {#each PLACES as p (p.href)}
          {@const Icon = p.icon}
          <SidebarMenuItem>
            <SidebarMenuButton
              id={`rail-${p.href === "/" ? "browse" : p.href.slice(1)}`}
              isActive={current === p.href}
              onclick={() => goto(p.href)}
            >
              <Icon strokeWidth={1.75} />
              <span>{$t(p.labelKey)}</span>
              {#if p.href === "/updates" && $updateCount > 0}
                <span class="ms-auto text-xs tabular-nums text-sidebar-foreground/55 group-data-[collapsible=icon]:hidden">
                  {$updateCount}
                </span>
              {/if}
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
    </SidebarGroup>
  </SidebarContent>
  <SidebarRail />
</Sidebar>
