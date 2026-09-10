<script lang="ts">
	import { Tooltip } from "bits-ui";
	import type { Snippet } from "svelte";

	let {
		children,
		instant = false,
		suppressed = false,
	}: {
		children?: Snippet;
		/** When true, tooltip appears immediately (0ms). Default: 1500ms. */
		instant?: boolean;
		/**
		 * Hold the tooltip shut while something else is on screen - a panel this
		 * trigger opened, say. A `suppressed` flag rather than a bindable `open`
		 * because almost every caller wants the tooltip to govern itself, and
		 * `bind:open={undefined}` against a prop with a fallback is a runtime
		 * error in Svelte 5 (measured: it took the whole shell blank).
		 */
		suppressed?: boolean;
	} = $props();

	const delay = $derived(instant ? 0 : 1500);

	// Our own state, bound both ways: bits-ui opens and closes it on hover, and
	// this closes it again the moment it is opened while suppressed - which is
	// what makes the flag hold rather than apply once.
	let openState = $state(false);
	$effect(() => {
		if (suppressed && openState) openState = false;
	});
</script>

<!--
  Tooltip root. Default 1500ms delay for informational tooltips.
  Pass instant={true} for elements where the tooltip confirms an
  action target (e.g. workspace pills).
-->
<Tooltip.Provider>
	<Tooltip.Root delayDuration={delay} bind:open={openState}>
		{@render children?.()}
	</Tooltip.Root>
</Tooltip.Provider>
