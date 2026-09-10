<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { setContext } from "svelte";
	import { cn } from "../../../utils.js";
	import { COMMAND_LIST_ID } from "./context.js";

	// ONE ID PER PALETTE, minted here and handed to the list (which wears it) and
	// the input (which points at it with `aria-controls`). A page can hold two
	// palettes - the terminal mounts history AND quick-connect - so a fixed id
	// would have the input of one pointing at the list of the other.
	setContext(COMMAND_LIST_ID, `arlen-command-list-${Math.random().toString(36).slice(2, 9)}`);

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
