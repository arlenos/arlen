<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";
	import { Search } from "lucide-svelte";

	let {
		ref = $bindable(null),
		value = $bindable(""),
		class: className,
		...restProps
	}: CommandPrimitive.InputProps = $props();
</script>

<div
	class="flex items-center gap-2 px-3"
	style="border-bottom: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);"
	data-slot="command-input-wrapper"
>
	<Search class="size-4 shrink-0" style="color: color-mix(in srgb, var(--color-fg-shell) 40%, transparent);" />
	<CommandPrimitive.Input
		bind:ref
		bind:value
		data-slot="command-input"
		class={cn(
			"flex h-10 w-full rounded-md bg-transparent py-3 text-sm outline-hidden disabled:cursor-not-allowed disabled:opacity-50",
			className
		)}
		style="color: var(--color-fg-shell); --placeholder-color: color-mix(in srgb, var(--color-fg-shell) 35%, transparent);"
		{...restProps}
	/>
</div>
