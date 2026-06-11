<script lang="ts">
  /// The quiet capability line under the composer, anchored by a
  /// status dot: the same sentence pattern as the harness strip
  /// (copy law: plain words on the surface, the technical facts live
  /// in the tooltip). A failed read renders as unreachable with a
  /// retry, never as "off".
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { readCapability, type Capability } from "$lib/contract";

  const capability = writable<Capability | null>(null);
  const loaded = writable(false);

  async function load() {
    capability.set(await readCapability());
    loaded.set(true);
  }
  onMount(load);

  function sentence(c: Capability): string {
    if (!c.enabled) return "AI is off. You can turn it on in Settings.";
    return c.executorLive
      ? "AI is on. It can make small changes you can undo."
      : "AI is on. It only suggests changes.";
  }

  /// Lay sentences for the known read tiers; unknown tiers omit the
  /// clause rather than inventing one.
  const TIER_SENTENCES: Record<string, string> = {
    none: "It cannot see your files.",
    metadata: "It sees file names and dates, not what is inside files.",
    structural: "It sees file names and dates, not what is inside files.",
    content: "It can read your files.",
    full: "It can read your files.",
  };

  function facts(c: Capability): string {
    const parts: string[] = [];
    const tier = TIER_SENTENCES[c.tier?.toLowerCase?.() ?? ""];
    if (tier) parts.push(tier);
    const model = [c.provider, c.model].filter(Boolean).join(" ");
    if (model) parts.push(`Model: ${model}. Change this in Settings.`);
    return parts.join(" ");
  }
</script>

{#if $loaded}
  <div class="strip">
    {#if $capability}
      {#if facts($capability)}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <p {...props} class="line">
                <span class="strip-dot" class:off={!$capability?.enabled} aria-hidden="true"></span>
                {sentence($capability)}
              </p>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.TooltipContent side="top">
            {facts($capability)}
          </Tooltip.TooltipContent>
        </Tooltip.Root>
      {:else}
        <p class="line">
          <span class="strip-dot" class:off={!$capability?.enabled} aria-hidden="true"></span>
          {sentence($capability)}
        </p>
      {/if}
    {:else}
      <p class="line">
        <span class="strip-dot unreachable" aria-hidden="true"></span>
        Can't reach the assistant right now.
      </p>
      <Button variant="outline" size="sm" onclick={load}>Try again</Button>
    {/if}
  </div>
{/if}

<style>
  /* A fixed minimum keeps the composer from shifting when the state
     swaps the sentence for the retry row. */
  .strip {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 8px;
    min-height: var(--height-control, 28px);
    min-width: 0;
  }
  .line {
    margin: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The one dot language (fills, outside the text dim scale):
     green on, gray off, red unreachable. */
  .strip-dot {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-success);
  }
  .strip-dot.off {
    background: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .strip-dot.unreachable {
    background: var(--color-error);
  }
</style>
