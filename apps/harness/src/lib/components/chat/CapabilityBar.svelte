<script lang="ts">
  /// The quiet status line under the composer: capability and posture as one
  /// plain sentence, anchored by the capability glyph. The technical facts
  /// (model, provider, per-question independence) live in the tooltip. This
  /// is the in-body capability strip; nothing about it lives in the header.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import type { Capability } from "$lib/capability";
  import { statusSentence, statusTooltip } from "$lib/display";

  let {
    capability,
    loaded,
    onretry,
  }: {
    /// The capability read; `null` after a failed read.
    capability: Capability | null;
    /// False until the first read settles, so nothing flashes.
    loaded: boolean;
    onretry: () => void;
  } = $props();
</script>

{#if loaded}
  <div class="status">
    {#if capability}
      <p class="line" title={statusTooltip(capability)}>
        <span class="glyph" class:off={!capability.enabled} aria-hidden="true">◆</span>
        {statusSentence(capability)}
      </p>
    {:else}
      <p class="line">Can't reach the assistant right now.</p>
      <Button variant="outline" size="sm" onclick={onretry}>Try again</Button>
    {/if}
  </div>
{/if}

<style>
  .status {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
    margin-top: 0.5rem;
    min-width: 0;
  }
  .line {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .glyph {
    color: var(--color-success);
    margin-right: 0.25rem;
  }
  .glyph.off {
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
</style>
