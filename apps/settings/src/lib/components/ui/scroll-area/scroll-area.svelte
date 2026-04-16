<script lang="ts">
	import { ScrollArea as ScrollAreaPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		orientation = "vertical",
		children,
		...restProps
	}: ScrollAreaPrimitive.RootProps & { orientation?: "vertical" | "horizontal" } = $props();
</script>

<ScrollAreaPrimitive.Root
	bind:ref
	class={cn("relative overflow-hidden", className)}
	{...restProps}
>
	<ScrollAreaPrimitive.Viewport class="h-full w-full rounded-[inherit]">
		{@render children?.()}
	</ScrollAreaPrimitive.Viewport>
	<ScrollAreaPrimitive.Scrollbar
		{orientation}
		class="flex touch-none select-none transition-colors data-[orientation=horizontal]:h-2.5 data-[orientation=vertical]:w-2.5 data-[orientation=horizontal]:flex-col data-[orientation=vertical]:border-l data-[orientation=horizontal]:border-t border-transparent p-px"
	>
		<ScrollAreaPrimitive.Thumb
			class="bg-border relative flex-1 rounded-full"
		/>
	</ScrollAreaPrimitive.Scrollbar>
</ScrollAreaPrimitive.Root>
