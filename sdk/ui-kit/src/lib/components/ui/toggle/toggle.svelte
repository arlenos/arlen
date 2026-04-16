<script lang="ts">
	import { Toggle as TogglePrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		pressed = $bindable(false),
		class: className,
		...restProps
	}: TogglePrimitive.RootProps = $props();
</script>

<TogglePrimitive.Root
	bind:ref
	bind:pressed
	class={cn(
		"inline-flex items-center justify-center rounded-md text-sm font-medium transition-colors hover:bg-muted hover:text-muted-foreground data-[state=on]:bg-secondary data-[state=on]:text-secondary-foreground focus-visible:outline-none focus-visible:ring-1 disabled:pointer-events-none disabled:opacity-50",
		className
	)}
	{...restProps}
/>
