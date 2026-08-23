<script lang="ts">
  /// One notice line in the reading surface's three-tone language: error is a
  /// statement about whether the message below can be believed at all, caution
  /// is the message doing something worth your eyes (diverging parts, report-
  /// back headers), neutral is a fact stated once (sealed, HTML withheld, a
  /// named calendar part). The copy comes in as a finished sentence - the tone
  /// only shapes it.
  import { Info, OctagonAlert, TriangleAlert } from "@lucide/svelte";

  let { tone, text }: { tone: "error" | "caution" | "neutral"; text: string } = $props();
</script>

<p class="notice {tone}" role={tone === "error" ? "alert" : "status"}>
  {#if tone === "error"}
    <OctagonAlert size={14} strokeWidth={2} aria-hidden="true" />
  {:else if tone === "caution"}
    <TriangleAlert size={14} strokeWidth={2} aria-hidden="true" />
  {:else}
    <Info size={14} strokeWidth={2} aria-hidden="true" />
  {/if}
  <span>{text}</span>
</p>

<style>
  .notice {
    display: flex;
    gap: 0.55rem;
    align-items: flex-start;
    margin: 0;
    padding: 0.5rem 0.7rem;
    border-inline-start: 2px solid transparent;
    border-radius: var(--radius-input, 8px);
    font-size: var(--text-xs, 12px);
    line-height: 1.5;
  }
  .notice :global(svg) {
    flex-shrink: 0;
    margin-top: 0.15rem;
  }
  .error {
    border-color: var(--color-error, #dc2626);
    background: color-mix(in srgb, var(--color-error, #dc2626) 8%, transparent);
    color: var(--color-fg-primary);
  }
  .caution {
    border-color: var(--color-fg-warning, #eab308);
    background: color-mix(in srgb, var(--color-fg-warning, #eab308) 7%, transparent);
    color: var(--color-fg-primary);
  }
  .neutral {
    border-color: color-mix(in srgb, var(--color-fg-primary) 25%, transparent);
    background: color-mix(in srgb, var(--color-fg-primary) 3%, transparent);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
</style>
