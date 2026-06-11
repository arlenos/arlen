<script lang="ts">
  /// The places groups for a browser sidebar: rows with semantic
  /// place icons (Home, Downloads, USB carry meaning, unlike
  /// decorative per-row icons), one text edge, the dot language for
  /// mount state (gray = offline). The host wraps this in its own
  /// Sidebar shell and decides which groups exist.
  import {
    SidebarGroup,
    SidebarGroupLabel,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
  } from "../ui/sidebar";
  import { placeIcon } from "./icons";
  import type { Place, PlaceGroup } from "./types";

  let {
    groups,
    activePath,
    onnavigate,
  }: {
    groups: PlaceGroup[];
    /// The current location; the matching place row renders active.
    activePath?: string;
    onnavigate?: (place: Place) => void;
  } = $props();
</script>

{#each groups as group (group.label)}
  {#if group.places.length > 0}
    <SidebarGroup>
      <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
      <SidebarMenu>
        {#each group.places as place (place.path)}
          {@const Icon = placeIcon(place.icon)}
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={activePath === place.path}
              tooltip={place.offline ? `${place.label} (not connected)` : place.path}
              onclick={() => onnavigate?.(place)}
            >
              <Icon />
              <span class="ps-label" class:offline={place.offline}>
                {place.label}
              </span>
              {#if place.offline}
                <span class="ps-dot ml-auto group-data-[collapsible=icon]:hidden"></span>
              {/if}
            </SidebarMenuButton>
          </SidebarMenuItem>
        {/each}
      </SidebarMenu>
    </SidebarGroup>
  {/if}
{/each}

<style>
  .ps-label {
    font-size: 0.8125rem;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ps-label.offline {
    color: color-mix(in srgb, var(--sidebar-foreground) 55%, transparent);
  }
  /* The one dot language: gray = not connected. */
  .ps-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--sidebar-foreground) 30%, transparent);
    flex-shrink: 0;
  }
</style>
