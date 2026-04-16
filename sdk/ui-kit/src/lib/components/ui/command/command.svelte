<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		value = $bindable(""),
		class: className,
		...restProps
	}: CommandPrimitive.RootProps = $props();
</script>

<CommandPrimitive.Root
	bind:ref
	bind:value
	data-slot="command"
	class={cn(
		"flex h-full w-full flex-col overflow-hidden rounded-md",
		className
	)}
	style="background: var(--color-bg-shell); color: var(--color-fg-shell);"
	{...restProps}
/>
