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
    SidebarHeader,
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
  <SidebarHeader class="h-10 flex-row items-center py-0">
    <span
      class="px-2 text-[0.6875rem] font-semibold uppercase tracking-[0.1em] text-sidebar-foreground/55 group-data-[collapsible=icon]:hidden"
    >
      {$t("st.title")}
    </span>
  </SidebarHeader>
  <SidebarContent>
    <SidebarGroup>
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
