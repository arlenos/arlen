<script lang="ts">
	/// The disclosed content. Motion (design-system §6b): the act is opening,
	/// so the content unfolds to its own height and folds back when closed,
	/// on `--duration-normal`, which the theme zeroes under reduce motion so
	/// the content then simply appears. bits-ui measures the height into
	/// `--bits-collapsible-content-height` and waits for the closing animation
	/// before it unmounts.
	import { Collapsible as CollapsiblePrimitive } from "bits-ui";
	import { cn } from "../../../utils.js";

	let { ref = $bindable(null), class: className, ...restProps }: CollapsiblePrimitive.ContentProps = $props();
</script>

<CollapsiblePrimitive.Content bind:ref data-slot="collapsible-content" class={cn("kit-disclosed", className)} {...restProps} />

<style>
	:global(.kit-disclosed) {
		overflow: hidden;
	}
	:global(.kit-disclosed[data-state="open"]) {
		animation: kit-unfold var(--duration-normal) var(--ease-out);
	}
	:global(.kit-disclosed[data-state="closed"]) {
		animation: kit-fold var(--duration-normal) var(--ease-out);
	}
	@keyframes -global-kit-unfold {
		from {
			height: 0;
			opacity: 0;
		}
		to {
			height: var(--bits-collapsible-content-height);
			opacity: 1;
		}
	}
	@keyframes -global-kit-fold {
		from {
			height: var(--bits-collapsible-content-height);
			opacity: 1;
		}
		to {
			height: 0;
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		:global(.kit-disclosed[data-state]) {
			animation: none;
		}
	}
</style>
