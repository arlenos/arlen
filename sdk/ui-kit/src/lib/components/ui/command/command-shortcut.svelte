<script lang="ts">
	import { cn } from "../../../utils.js";
	import type { HTMLAttributes } from "svelte/elements";

	let {
		class: className,
		children,
		...restProps
	}: HTMLAttributes<HTMLSpanElement> & { children?: import("svelte").Snippet } = $props();
</script>

<!-- 55%, not 30%. The mix composites against whatever row it sits on, and the
     SELECTED row is the light one - on the shell's menu palette that made
     #737373 on #393939, which is 2.43:1 where AA wants 4.5. The text it carries
     is the key you would press, so it is the one part of the row a person has to
     be able to read on the row they are pointed at. Measured on the row that
     lightens most; a dimmer mix passes on the resting rows and fails there. -->
<span
	data-slot="command-shortcut"
	class={cn("ms-auto text-xs tracking-widest", className)}
	style="color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);"
	{...restProps}
>
	{@render children?.()}
</span>
