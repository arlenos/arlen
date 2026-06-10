<script lang="ts">
  /// System Explanation Mode (Foundation §5.8): an on-demand plain-language
  /// summary of what the computer is doing now. Rendering only; the page
  /// owns the explain call.
  import { Sparkles, Telescope } from "@lucide/svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { renderMarkdown } from "$lib/markdown";
  import { externalLinks } from "$lib/externalLinks";

  let {
    explanation,
    error,
    busy,
    onexplain,
  }: {
    explanation: string | null;
    error: string | null;
    busy: boolean;
    onexplain: () => void;
  } = $props();
</script>

<div class="explain">
  <p class="hint">
    <Telescope size={14} strokeWidth={1.75} />
    A plain-language summary of what your computer is doing right now, grounded in the knowledge
    graph, live processes and any flagged anomalies. Generated on demand.
  </p>
  <Button variant="outline" size="sm" disabled={busy} onclick={onexplain}>
    <Sparkles size={14} class={busy ? "spin" : ""} />
    {busy ? "Thinking…" : "Explain"}
  </Button>
  {#if error}
    <p class="error">{error}</p>
  {:else if explanation}
    <!-- Model prose (markdown); rendered the same sanitized way as chat
         answers. -->
    <div class="text markdown" use:externalLinks>{@html renderMarkdown(explanation)}</div>
  {/if}
</div>

<style>
  .explain {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.625rem;
    padding: 0.5rem var(--space-row, 0.75rem) 0.75rem;
  }
  .hint {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .hint :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
  }
  .text {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.55;
    color: var(--foreground);
  }
  .error {
    margin: 0;
    font-size: 0.8125rem;
    color: var(--color-error);
  }
</style>
