<script lang="ts">
  import { page } from "$app/stores";
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
  } from "$lib/components/ui/sidebar";
  import { PANELS, navigateTo } from "$lib/stores/navigation";
  import {
    Settings2,
    Palette,
    Keyboard,
    Command,
    Monitor,
    Mouse,
    SquareMousePointer,
    Bell,
    Shield,
    Puzzle,
    Info,
  } from "lucide-svelte";

  const ICONS: Record<string, typeof Palette> = {
    Palette,
    Keyboard,
    Command,
    Monitor,
    Mouse,
    SquareMousePointer,
    Bell,
    Shield,
    Puzzle,
    Info,
  };

  // Group panels by section, matching the settings-app spec.
  const SECTIONS = [
    {
      label: "Personalization",
      panelIds: ["appearance", "display"] as const,
    },
    {
      label: "Input",
      panelIds: ["keyboard", "shortcuts", "mouse", "touchpad"] as const,
    },
    {
      label: "System",
      panelIds: ["notifications", "privacy", "extensions"] as const,
    },
    {
      label: "Info",
      panelIds: ["about"] as const,
    },
  ];

  function isActive(href: string): boolean {
    return $page.url.pathname === href;
  }
</script>

<Sidebar collapsible="icon">
  <SidebarHeader>
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton size="lg" tooltip="Lunaris Settings">
          <div
            class="flex aspect-square size-8 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground"
          >
            <Settings2 size={16} />
          </div>
          <div class="flex flex-col gap-0.5 leading-none">
            <span class="font-medium">Settings</span>
            <span class="text-xs text-sidebar-foreground/60">Lunaris OS</span>
          </div>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  </SidebarHeader>

  <SidebarContent>
    {#each SECTIONS as section (section.label)}
      <SidebarGroup>
        <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
        <SidebarMenu>
          {#each section.panelIds as id}
            {@const panel = PANELS.find((p) => p.id === id)}
            {#if panel}
              {@const Icon = ICONS[panel.icon] ?? Palette}
              <SidebarMenuItem>
                <SidebarMenuButton
                  isActive={isActive(panel.href)}
                  disabled={!panel.enabled}
                  tooltip={panel.title}
                  onclick={() => navigateTo(panel.id)}
                >
                  <Icon />
                  <span>{panel.title}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            {/if}
          {/each}
        </SidebarMenu>
      </SidebarGroup>
    {/each}
  </SidebarContent>

  <SidebarRail />
</Sidebar>
