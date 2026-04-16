<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		heading,
		...restProps
	}: CommandPrimitive.GroupProps & { heading?: string } = $props();
</script>

<CommandPrimitive.Group
	bind:ref
	data-slot="command-group"
	class={cn("overflow-hidden p-1", className)}
	style="color: var(--color-fg-shell);"
	{...restProps}
>
	{#if heading}
		<CommandPrimitive.GroupHeading
			class="px-2 py-1.5 text-xs font-medium"
			style="color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);"
			data-slot="command-group-heading"
		>
			{heading}
		</CommandPrimitive.GroupHeading>
	{/if}
	<CommandPrimitive.GroupItems>
		{@render restProps.children?.()}
	</CommandPrimitive.GroupItems>
</CommandPrimitive.Group>
