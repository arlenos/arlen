<script lang="ts">
	import { Tooltip } from "bits-ui";
	import type { Snippet } from "svelte";

	let { children }: { children?: Snippet } = $props();
</script>

<!--
  Tooltip root with the 400ms delay defined in the Lunaris interaction design system.
  Never override delayDuration per-instance - it is a global standard.
-->
<Tooltip.Provider>
	<Tooltip.Root delayDuration={400}>
		{@render children?.()}
	</Tooltip.Root>
</Tooltip.Provider>
