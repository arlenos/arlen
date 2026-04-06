<script lang="ts">
	import { cn } from "$lib/utils.js";
	import type { HTMLAttributes } from "svelte/elements";

	let {
		class: className,
		variant = "default",
		children,
		...restProps
	}: HTMLAttributes<HTMLSpanElement> & {
		variant?: "default" | "secondary" | "destructive" | "outline";
		children?: import("svelte").Snippet;
	} = $props();

	const variants: Record<string, string> = {
		default: "bg-primary text-primary-foreground",
		secondary: "bg-secondary text-secondary-foreground",
		destructive: "bg-destructive text-destructive-foreground",
		outline: "border border-border text-foreground",
	};
</script>

<span
	class={cn(
		"inline-flex items-center rounded-full px-2 py-0.5 text-xs font-semibold transition-colors",
		variants[variant],
		className
	)}
	{...restProps}
>
	{@render children?.()}
</span>
