<script lang="ts">
  /// Themed tooltip body (portal included): the canonical look for label
  /// tooltips on icon-only controls. The raw bits-ui `Content` stays
  /// exported for surfaces that bring their own styling (the topbar
  /// Applet).
  import { Tooltip as TooltipPrimitive } from "bits-ui";
  import { cn } from "$lib/utils";

  let {
    ref = $bindable(null),
    class: className,
    sideOffset = 6,
    children,
    ...restProps
  }: TooltipPrimitive.ContentProps & { sideOffset?: number } = $props();
</script>

<TooltipPrimitive.Portal>
  <TooltipPrimitive.Content
    bind:ref
    {sideOffset}
    data-slot="tooltip-content"
    class={cn(
      "z-50 rounded-button border border-border bg-popover px-2 py-1 text-xs text-popover-foreground shadow-md",
      className
    )}
    {...restProps}
  >
    {@render children?.()}
  </TooltipPrimitive.Content>
</TooltipPrimitive.Portal>
