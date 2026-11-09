<script lang="ts">
	// Avatars are round, one of the few things that are (halbat's three radii,
	// 9 November 2026). Everything clickable is 6, everything floating 12.

	import { Avatar as AvatarPrimitive } from "bits-ui";
	import { cn } from "../../../utils.js";

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: AvatarPrimitive.RootProps = $props();
</script>

<AvatarPrimitive.Root
	bind:ref
	class={cn(
		"relative flex size-8 shrink-0 overflow-hidden rounded-full",
		className
	)}
	{...restProps}
/>
