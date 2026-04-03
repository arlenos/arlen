<script lang="ts">
	import { Tooltip } from "bits-ui";
	import type { Snippet } from "svelte";

	let {
		children,
		instant = false,
	}: {
		children?: Snippet;
		/** When true, tooltip appears immediately (0ms). Default: 1500ms. */
		instant?: boolean;
	} = $props();

	const delay = $derived(instant ? 0 : 1500);
</script>

<!--
  Tooltip root. Default 1500ms delay for informational tooltips.
  Pass instant={true} for elements where the tooltip confirms an
  action target (e.g. workspace pills).
-->
<Tooltip.Provider>
	<Tooltip.Root delayDuration={delay}>
		{@render children?.()}
	</Tooltip.Root>
</Tooltip.Provider>
