<script lang="ts">
  /// Cost: the honest answer to "what does it cost." Local-first is the
  /// default: a provider on your own machine has no per-token cost, and
  /// the surface says so plainly. A cloud provider shows the running
  /// token count once that accounting lands; until then it says "not
  /// measured yet", never a fake zero. Rendering only.
  import type { Capability } from "$lib/capability";
  import { isLocalProvider, providerDisplay } from "$lib/transparency";
  import SectionState from "./SectionState.svelte";

  let {
    capability,
    loaded,
  }: {
    capability: Capability | null;
    loaded: boolean;
  } = $props();

  const off = $derived(capability !== null && !capability.enabled);
  const local = $derived(capability !== null && isLocalProvider(capability.provider));
  // The provider name in parentheses, named but secondary; the category
  // "a cloud service" leads so a layperson is not shown a bare brand id.
  const providerSuffix = $derived.by(() => {
    const name = providerDisplay(capability?.provider);
    return name ? ` (${name})` : "";
  });
</script>

{#if !loaded}
  <SectionState message="Checking how this is set up." />
{:else if capability === null}
  <SectionState message="Can't tell how this is set up right now." />
{:else if off}
  <SectionState tag="AI is off" tone="off" message="The AI is off, so it is costing nothing." />
{:else if local}
  <div class="cost">
    <p class="line">This assistant runs on your own computer.</p>
    <p class="sub">There is no usage cost.</p>
  </div>
{:else}
  <div class="cost">
    <p class="line">This assistant uses a cloud service{providerSuffix}.</p>
    <p class="sub">
      <span class="tag">Not measured yet</span>
      Cloud use has a cost. Arlen does not count it yet, so nothing is shown here.
    </p>
  </div>
{/if}

<style>
  .cost {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    padding: 0.75rem var(--space-row, 0.75rem);
  }
  .line {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .tag {
    display: inline-flex;
    align-items: center;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  }
</style>
