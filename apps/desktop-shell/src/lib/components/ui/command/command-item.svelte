<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: CommandPrimitive.ItemProps = $props();
</script>

<CommandPrimitive.Item
	bind:ref
	data-slot="command-item"
	class={cn(
		"command-item-shell [&_svg:not([class*='size-'])]:size-4 relative flex cursor-default gap-2 rounded-sm px-2 py-1.5 text-sm items-center outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0",
		className
	)}
	{...restProps}
/>

<style>
	:global(.command-item-shell) {
		color: var(--color-fg-shell);
	}
	:global(.command-item-shell:hover) {
		background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
	}
	:global(.command-item-shell[data-selected]) {
		background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent) !important;
		color: var(--color-fg-shell);
	}
</style>
