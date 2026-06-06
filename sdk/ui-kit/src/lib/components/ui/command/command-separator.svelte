<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: CommandPrimitive.SeparatorProps = $props();
</script>

<CommandPrimitive.Separator
	bind:ref
	data-slot="command-separator"
	class={cn("-mx-1 h-px", className)}
	style="background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);"
	{...restProps}
/>
