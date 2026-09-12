<script lang="ts">
	import { Command as CommandPrimitive } from "bits-ui";
	import { getContext } from "svelte";
	import { cn } from "../../../utils.js";
	import { COMMAND_LIST_ID } from "./context.js";
	import { Search } from "@lucide/svelte";

	let {
		ref = $bindable(null),
		value = $bindable(""),
		class: className,
		...restProps
	}: CommandPrimitive.InputProps = $props();

	// What this input controls. bits-ui gives it `role="combobox"` and leaves
	// `aria-controls` unset; see `context.ts`.
	const listId = getContext<string>(COMMAND_LIST_ID);

</script>

<div
	class="flex items-center gap-2 px-3"
	style="border-bottom: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);"
	data-slot="command-input-wrapper"
>
	<Search class="size-4 shrink-0" style="color: color-mix(in srgb, var(--color-fg-shell) 40%, transparent);" />
	<!-- NO FOCUS RING, and it is the one input in the kit without one, so here is
	     why before somebody adds it or reports it again. `Input` rings on
	     `:focus-visible`; this one is `outline-hidden` like upstream shadcn's,
	     because a palette opens WITH this focused and nothing else in it is
	     focusable until the arrow keys move a cursor into the list - the ring
	     would be a permanent frame round the only place the keyboard can be.

	     `no-focus-ring.js` reports it at every width, on every surface that opens
	     a palette - which is more than the two this comment used to name. A 720
	     German sweep on 13 September found it on the terminal's history and
	     quick-connect palettes, the shell's waypointer and the shell's menu
	     palette, and the clipboard popover uses the same input. Naming surfaces
	     here was the mistake: a kit component should not have to track its own
	     consumers, and the list was already stale. The probe is not wrong either:
	     it cannot know that this input is the whole widget. Left as it is rather
	     than silenced, and this comment is the answer to the finding. If the house
	     rule turns out to be that every input rings, this is the single place to
	     change. -->
	<CommandPrimitive.Input
		bind:ref
		bind:value
		aria-controls={listId}
		data-slot="command-input"
		class={cn(
			"flex h-10 w-full rounded-input bg-transparent py-3 text-sm outline-hidden disabled:opacity-50",
			className
		)}
		style="color: var(--color-fg-shell); --placeholder-color: color-mix(in srgb, var(--color-fg-shell) 35%, transparent);"
		{...restProps}
	/>
</div>
