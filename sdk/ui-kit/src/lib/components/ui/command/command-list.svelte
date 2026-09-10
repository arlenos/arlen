<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { getContext } from "svelte";
	import { cn } from "../../../utils.js";
	import { COMMAND_LIST_ID } from "./context.js";

	// The id the palette's input points at with `aria-controls`. Rendered rather
	// than set imperatively: an attribute put on with `setAttribute` is wiped the
	// next time Svelte reconciles this element, which for a palette is every
	// keystroke - measured, and the reason the first cut of this looked like it
	// did nothing at all.
	const listId = getContext<string>(COMMAND_LIST_ID);

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: CommandPrimitive.ListProps = $props();

</script>

<CommandPrimitive.List
	bind:ref
	id={listId}
	data-slot="command-list"
	class={cn("overflow-x-hidden overflow-y-auto", className)}
	{...restProps}
/>
