<script lang="ts">
  /// The harness sidebar: the two primary surfaces of the AI app
  /// (ai-app.md §2) — Conversation (query mode) and Agent (pull /
  /// observability). Built on the `@lunaris/ui-kit` sidebar canon, the
  /// same component Settings uses, so the app is structurally
  /// consistent with the rest of the OS.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import {
    Sidebar,
    SidebarHeader,
    SidebarContent,
    SidebarGroup,
    SidebarMenu,
    SidebarMenuItem,
    SidebarMenuButton,
    SidebarRail,
  } from "@lunaris/ui-kit/components/ui/sidebar";
  import { MessageSquare, Activity, Sparkles } from "@lucide/svelte";

  interface Surface {
    href: string;
    title: string;
    icon: typeof MessageSquare;
  }

  const SURFACES: Surface[] = [
    { href: "/", title: "Conversation", icon: MessageSquare },
    { href: "/agent", title: "Agent", icon: Activity },
  ];

  // Active when the path matches exactly (root) or is a sub-path.
  function isActive(href: string, path: string): boolean {
    return href === "/" ? path === "/" : path.startsWith(href);
  }
</script>

<Sidebar collapsible="icon">
  <SidebarHeader>
    <div class="harness-brand">
      <Sparkles size={18} strokeWidth={1.75} />
      <span class="harness-brand-text">AI</span>
    </div>
  </SidebarHeader>

  <SidebarContent>
    <SidebarGroup>
      <SidebarMenu>
        {#each SURFACES as surface (surface.href)}
          {@const Icon = surface.icon}
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={isActive(surface.href, $page.url.pathname)}
              tooltip={surface.title}
              onclick={() => goto(surface.href)}
            >
              <Icon size={16} strokeWidth={1.75} />
              <span>{surface.title}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
    </SidebarGroup>
  </SidebarContent>

  <SidebarRail />
</Sidebar>

<style>
  .harness-brand {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.5rem 0.25rem;
    color: var(--color-accent);
  }
  .harness-brand-text {
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--foreground);
  }
</style>
