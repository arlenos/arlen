<script lang="ts" module>
	import { type VariantProps, tv } from "tailwind-variants";

	export const badgeVariants = tv({
		// `text-xs/none` is load-bearing, not tidying. The badge's height is fixed
		// at `--height-tag` (20px) and its content was taller than that: 13px text
		// at the theme's line-height ratio is 17.33, plus `py-0.5` top and bottom
		// makes 21.33 in a 20px box with `overflow-hidden`, so every badge in the
		// system clipped its own descenders by about a pixel. Measured in the
		// browser on 9 September - `clientHeight` 19, `scrollHeight` 20 - after the
		// render sweep flagged `tall-cut 18<20` on every page that draws one.
		// Collapsing the line box to the font size leaves 13 + 4 in 20 and the
		// centring does the rest, so nothing moves and nothing is cut.
		//
		// As the size's own modifier rather than a `leading-none` beside it: in
		// Tailwind v4 `text-xs` sets font-size AND line-height, and two utilities
		// in one layer are resolved by generated order rather than by the order
		// they were typed - so the separate class measured as no change at all.
		base: "h-tag gap-1 rounded-chip border border-transparent px-2 py-0.5 text-xs/none font-medium has-data-[icon=inline-end]:pe-1.5 has-data-[icon=inline-start]:ps-1.5 [&>svg]:size-3! focus-visible:border-ring focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive group/badge inline-flex w-fit shrink-0 items-center justify-center overflow-hidden whitespace-nowrap transition-[color,background-color,border-color,transform] duration-fast ease-default focus-visible:ring-[3px] [&>svg]:pointer-events-none [a]:active:scale-[0.97] [a]:active:duration-instant",
		variants: {
			variant: {
				default: "bg-primary text-primary-foreground [a]:hover:bg-primary/80",
				secondary: "bg-secondary text-secondary-foreground [a]:hover:bg-secondary/80",
				destructive: "bg-destructive/10 [a]:hover:bg-destructive/20 focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40 text-destructive dark:bg-destructive/20",
				success: "bg-[color-mix(in_srgb,var(--color-success)_14%,transparent)] text-[var(--color-success)]",
				warn: "bg-[color-mix(in_srgb,var(--color-warning)_16%,transparent)] text-[var(--color-warning)]",
				outline: "bg-[color-mix(in_srgb,var(--foreground)_8%,transparent)] text-[color-mix(in_srgb,var(--foreground)_75%,transparent)] [a]:hover:bg-[color-mix(in_srgb,var(--foreground)_13%,transparent)]",
				ghost: "hover:bg-muted hover:text-muted-foreground dark:hover:bg-muted/50",
				link: "text-primary underline-offset-4 hover:underline",
			},
		},
		defaultVariants: {
			variant: "default",
		},
	});

	export type BadgeVariant = VariantProps<typeof badgeVariants>["variant"];
</script>

<script lang="ts">
	import type { HTMLAnchorAttributes } from "svelte/elements";
	import { cn, type WithElementRef } from "../../../utils.js";

	let {
		ref = $bindable(null),
		href,
		class: className,
		variant = "default",
		children,
		...restProps
	}: WithElementRef<HTMLAnchorAttributes> & {
		variant?: BadgeVariant;
	} = $props();
</script>

<svelte:element
	this={href ? "a" : "span"}
	bind:this={ref}
	data-slot="badge"
	{href}
	class={cn(badgeVariants({ variant }), className)}
	{...restProps}
>
	{@render children?.()}
</svelte:element>
