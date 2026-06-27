<script lang="ts">
  /// Cost: the honest answer to "what does it cost." Local-first is the
  /// default: a provider on your own machine has no per-token cost, and
  /// the surface says so plainly. A cloud provider shows the running token
  /// count from `ai_usage`. Rendering only.
  import type { Capability } from "$lib/capability";
  import { isLocalProvider, providerDisplay } from "$lib/transparency";
  import SectionState from "./SectionState.svelte";

  /// Cumulative token usage from `ai_usage`, or null while loading / on a
  /// failed read.
  export interface Usage {
    totalTokens: number;
  }

  let {
    capability,
    usage,
    loaded,
  }: {
    capability: Capability | null;
    usage: Usage | null;
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
  // A grouped token count, e.g. "12,340 tokens".
  const tokenLine = $derived.by(() => {
    if (usage === null) return null;
    return `${usage.totalTokens.toLocaleString()} ${usage.totalTokens === 1 ? "token" : "tokens"} used so far`;
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
    <p class="sub">There is no usage cost{tokenLine ? `. ${tokenLine}.` : "."}</p>
  </div>
{:else}
  <div class="cost">
    <p class="line">This assistant uses a cloud service{providerSuffix}.</p>
    {#if tokenLine}
      <p class="sub">{tokenLine}. Cloud use has a cost; check your provider for the rate.</p>
    {:else}
      <p class="sub">
        <span class="tag">Not measured yet</span>
        Cloud use has a cost. The running count is not available right now.
      </p>
    {/if}
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
