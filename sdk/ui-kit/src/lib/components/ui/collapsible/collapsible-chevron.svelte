<script lang="ts">
	/// The disclosure's chevron, turning with its trigger: put it inside a
	/// `CollapsibleTrigger` and it follows the trigger's open state, so a
	/// caller no longer writes its own rotate rule. `from="right"` turns a
	/// quarter to point down (a settings expander), `from="down"` turns half to
	/// point up (a thread or a tool call). On `--duration-normal`, like the
	/// content it opens.
	import { ChevronDown, ChevronRight } from "@lucide/svelte";

	let { from = "right", size = 15 }: { from?: "right" | "down"; size?: number } = $props();
</script>

<span class="kit-chevron {from}" aria-hidden="true">
	{#if from === "right"}
		<ChevronRight {size} strokeWidth={2} />
	{:else}
		<ChevronDown {size} strokeWidth={2} />
	{/if}
</span>

<style>
	.kit-chevron {
		display: inline-flex;
		flex-shrink: 0;
		transition: rotate var(--duration-normal) var(--ease-out);
	}
	:global([data-slot="collapsible-trigger"][data-state="open"]) .kit-chevron.right {
		rotate: 90deg;
	}
	:global([data-slot="collapsible-trigger"][data-state="open"]) .kit-chevron.down {
		rotate: 180deg;
	}
	@media (prefers-reduced-motion: reduce) {
		.kit-chevron {
			transition: none;
		}
	}
</style>
