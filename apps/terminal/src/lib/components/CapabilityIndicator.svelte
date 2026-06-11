<script lang="ts">
  /// The capability indicator at the composer (terminal.md §4.4, dot
  /// treatment per Tim's June 2026 decision): a status dot on the
  /// right edge of the input box — green on, gray off, red
  /// unreachable. The sentence, the technical facts and the retry
  /// live in its popover, so the resting composer carries no prose.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
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
  <Popover.Root>
    <Popover.Trigger>
      {#snippet child({ props })}
        <button
          {...props}
          id="terminal-ai-indicator"
          class="cap-btn"
          aria-label="AI status"
        >
          <span
            class="cap-dot"
            class:off={$capability !== null && !$capability.enabled}
            class:unreachable={$capability === null}
          ></span>
        </button>
      {/snippet}
    </Popover.Trigger>
    <Popover.Content side="top" align="end" class="cap-pop">
      {#if $capability}
        <p class="cap-line">{sentence($capability)}</p>
        {#if facts($capability)}
          <p class="cap-facts">{facts($capability)}</p>
        {/if}
      {:else}
        <p class="cap-line">Can't reach the assistant right now.</p>
        <Button variant="outline" size="sm" onclick={load}>Try again</Button>
      {/if}
    </Popover.Content>
  </Popover.Root>
{/if}

<style>
  .cap-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-full);
    background: transparent;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .cap-btn:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  /* The one dot language (fills, outside the text dim scale):
     green on, gray off, red unreachable. */
  .cap-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-success);
  }
  .cap-dot.off {
    background: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .cap-dot.unreachable {
    background: var(--color-error);
  }

  :global(.cap-pop) {
    width: 280px;
    background: var(--color-bg-card);
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }
  .cap-line {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: var(--foreground);
  }
  .cap-facts {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
