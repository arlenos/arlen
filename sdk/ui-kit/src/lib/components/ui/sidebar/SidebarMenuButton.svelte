<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import { cn } from "$lib/utils";

  let {
    class: className,
    isActive = false,
    size = "default",
    tooltip,
    children,
    ...rest
  }: HTMLButtonAttributes & {
    isActive?: boolean;
    size?: "default" | "sm" | "lg";
    tooltip?: string;
    children?: Snippet;
  } = $props();

  const sizes = {
    sm: "h-7 text-xs",
    default: "h-8 text-sm",
    lg: "h-12 text-sm group-data-[collapsible=icon]:p-0!",
  } as const;
</script>

<button
  type="button"
  data-slot="sidebar-menu-button"
  data-sidebar="menu-button"
  data-size={size}
  data-active={isActive}
  aria-label={tooltip}
  class={cn(
    "peer/menu-button flex w-full items-center gap-2 overflow-hidden rounded-md p-2 text-left outline-hidden transition-[width,height,padding] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 data-[active=true]:bg-sidebar-accent data-[active=true]:font-medium data-[active=true]:text-sidebar-accent-foreground [&>svg]:size-4 [&>svg]:shrink-0",
    "group-data-[collapsible=icon]:size-8! group-data-[collapsible=icon]:p-2!",
    sizes[size],
    className
  )}
  {...rest}
>
  {@render children?.()}
</button>
