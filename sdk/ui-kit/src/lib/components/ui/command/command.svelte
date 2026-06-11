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

<!-- Surface colors belong to the consumer (the shell's waypointer
     paints itself via its .wp-root class) — a kit primitive must not
     hardcode shell tokens, and an inline style here would force every
     consumer override into !important. -->
<CommandPrimitive.Root
	bind:ref
	bind:value
	data-slot="command"
	class={cn(
		"flex h-full w-full flex-col overflow-hidden rounded-input",
		className
	)}
	{...restProps}
/>
