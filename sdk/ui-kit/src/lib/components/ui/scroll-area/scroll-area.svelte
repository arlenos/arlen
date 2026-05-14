<script lang="ts">
	import { ScrollArea as ScrollAreaPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		orientation = "vertical",
		type = "auto",
		children,
		...restProps
	}: ScrollAreaPrimitive.RootProps & { orientation?: "vertical" | "horizontal" } = $props();
</script>

<!-- type="auto" makes the scrollbar visible whenever content
     overflows (native-scrollbar convention). bits-ui defaults to
     "hover" which hides the indicator unless the user is hovering
     directly over the scrollable region. -->
<ScrollAreaPrimitive.Root
	bind:ref
	class={cn("ln-scroll-area", className)}
	{type}
	{...restProps}
>
	<ScrollAreaPrimitive.Viewport class="ln-scroll-viewport">
		{@render children?.()}
	</ScrollAreaPrimitive.Viewport>
	<ScrollAreaPrimitive.Scrollbar
		{orientation}
		class="ln-scroll-bar"
	>
		<ScrollAreaPrimitive.Thumb
			class="ln-scroll-thumb"
		/>
	</ScrollAreaPrimitive.Scrollbar>
</ScrollAreaPrimitive.Root>

<style>
	/* Plain CSS rather than Tailwind data-attribute variants because
	   the bits-ui data-orientation attribute lands at runtime and
	   Tailwind v4 utility generation can miss runtime-only data
	   variants in some build setups. */
	:global(.ln-scroll-area) {
		position: relative;
		overflow: hidden;
	}
	:global(.ln-scroll-viewport) {
		height: 100%;
		width: 100%;
		border-radius: inherit;
	}
	:global(.ln-scroll-bar) {
		display: flex;
		touch-action: none;
		user-select: none;
		padding: 2px;
		transition: background-color 150ms ease;
		background: color-mix(in srgb, var(--foreground) 4%, transparent);
	}
	:global(.ln-scroll-bar[data-orientation="vertical"]) {
		width: 10px;
		flex-direction: row;
	}
	:global(.ln-scroll-bar[data-orientation="horizontal"]) {
		height: 10px;
		flex-direction: column;
	}
	:global(.ln-scroll-bar:hover) {
		background: color-mix(in srgb, var(--foreground) 8%, transparent);
	}
	:global(.ln-scroll-thumb) {
		flex: 1;
		border-radius: 9999px;
		/* 38% gives visible contrast on both shell-popover (dark) and
		   app-surface (light) backgrounds — matches macOS / GNOME
		   scrollbar tone. */
		background: color-mix(in srgb, var(--foreground) 38%, transparent);
		min-height: 24px;
		min-width: 24px;
		transition: background-color 150ms ease;
	}
	:global(.ln-scroll-thumb:hover),
	:global(.ln-scroll-bar:active .ln-scroll-thumb) {
		background: color-mix(in srgb, var(--foreground) 55%, transparent);
	}
</style>
