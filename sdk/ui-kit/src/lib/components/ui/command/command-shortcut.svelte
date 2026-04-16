<script lang="ts">
	import { cn } from "$lib/utils.js";
	import type { HTMLAttributes } from "svelte/elements";

	let {
		class: className,
		children,
		...restProps
	}: HTMLAttributes<HTMLSpanElement> & { children?: import("svelte").Snippet } = $props();
</script>

<span
	data-slot="command-shortcut"
	class={cn("ml-auto text-xs tracking-widest", className)}
	style="color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent);"
	{...restProps}
>
	{@render children?.()}
</span>
