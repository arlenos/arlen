<script lang="ts">
	/// The dropdown's floating panel.
	///
	/// `overflow-x-hidden` is load-bearing and was the one missing from this
	/// content while the context menu and the command list both carry it. CSS
	/// pairs the axes: set `overflow-y` to `auto` and leave `overflow-x` at
	/// `visible` and the computed `overflow-x` becomes `auto` too - so any menu
	/// whose caller pins a width narrower than its own items turned into a
	/// HORIZONTALLY scrollable region with no way to scroll it, since every menu
	/// item sits at `tabindex="-1"`. Measured on the terminal's new-session menu:
	/// 224px of content in a 208px box, which axe rates serious and a keyboard
	/// user simply cannot reach the end of.
	import { cn, type WithoutChildrenOrChild } from "../../../utils.js";
	import DropdownMenuPortal from "./dropdown-menu-portal.svelte";
	import { DropdownMenu as DropdownMenuPrimitive } from "bits-ui";
	import type { ComponentProps } from "svelte";

	let {
		ref = $bindable(null),
		sideOffset = 4,
		align = "start",
		portalProps,
		class: className,
		...restProps
	}: DropdownMenuPrimitive.ContentProps & {
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof DropdownMenuPortal>>;
	} = $props();
</script>

<DropdownMenuPortal {...portalProps}>
	<DropdownMenuPrimitive.Content
		bind:ref
		data-slot="dropdown-menu-content"
		{sideOffset}
		{align}
		class={cn(
			"data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 ring-foreground/10 bg-popover text-popover-foreground min-w-32 max-w-[min(22rem,90vw)] rounded-input p-1 shadow-md ring-1 duration-100 data-[side=inline-start]:slide-in-from-right-2 data-[side=inline-end]:slide-in-from-left-2 z-50 w-max overflow-x-hidden overflow-y-auto outline-none data-closed:overflow-hidden",
			className
		)}
		{...restProps}
	/>
</DropdownMenuPortal>
