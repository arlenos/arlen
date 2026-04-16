<script lang="ts">
	import { Slider as SliderPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		value = $bindable(50),
		max = 100,
		min = 0,
		step = 1,
		class: className,
		onValueChange,
		...restProps
	}: {
		ref?: HTMLElement | null;
		value?: number;
		max?: number;
		min?: number;
		step?: number;
		class?: string;
		onValueChange?: (v: number) => void;
		[key: string]: unknown;
	} = $props();
</script>

<SliderPrimitive.Root
	bind:ref
	bind:value
	type="single"
	{max}
	{min}
	{step}
	{onValueChange}
	class={cn("relative flex w-full touch-none items-center select-none", className)}
>
	<span class="bg-muted relative h-1 w-full grow overflow-hidden rounded-full">
		<SliderPrimitive.Range class="bg-foreground absolute h-full opacity-40" />
	</span>
	<SliderPrimitive.Thumb
		index={0}
		class="border-foreground/50 bg-foreground block size-3 rounded-md border shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 disabled:pointer-events-none disabled:opacity-50"
	/>
</SliderPrimitive.Root>
