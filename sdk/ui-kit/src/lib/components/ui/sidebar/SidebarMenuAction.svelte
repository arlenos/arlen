<script lang="ts">
  /// Secondary action on a sidebar menu item (rename, delete, a "…" menu),
  /// absolutely positioned at the row's right edge. With `showOnHover` it
  /// stays invisible until the row is hovered or focused, so dense lists
  /// stay calm. Pair with a right padding on the `SidebarMenuButton`
  /// (e.g. `pe-7`) so long labels truncate under it.
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import { cn } from "$lib/utils";

  let {
    class: className,
    showOnHover = false,
    children,
    ...rest
  }: HTMLButtonAttributes & {
    showOnHover?: boolean;
    children?: Snippet;
  } = $props();
</script>

<button
  type="button"
  data-slot="sidebar-menu-action"
  data-sidebar="menu-action"
  class={cn(
    "absolute top-1.5 right-1 flex aspect-square w-5 items-center justify-center rounded-chip p-0 text-sidebar-foreground outline-hidden transition-opacity hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring [&>svg]:size-4 [&>svg]:shrink-0",
    "peer-data-[size=sm]/menu-button:top-1 peer-data-[size=default]/menu-button:top-1.5 peer-data-[size=lg]/menu-button:top-2.5",
    "group-data-[collapsible=icon]:hidden",
    showOnHover &&
      "opacity-0 group-focus-within/menu-item:opacity-100 group-hover/menu-item:opacity-100 data-[state=open]:opacity-100",
    className
  )}
  {...rest}
>
  {@render children?.()}
</button>
