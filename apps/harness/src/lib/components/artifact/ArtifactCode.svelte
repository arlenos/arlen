<script lang="ts">
  /// A code artifact body: plain monospace first, upgraded to Shiki-highlighted
  /// HTML (the muted Arlen theme) when it resolves. `data-selectable` re-enables
  /// text selection (the app disables it globally), so a range is drag-selectable
  /// + Ctrl+C-copyable. Dynamic height: as tall as the code up to a ceiling, then
  /// scrolls in place.
  import { highlightCode } from "./highlight";

  let { source, language }: { source: string; language?: string } = $props();

  let html = $state<string | null>(null);
  $effect(() => {
    const code = source;
    const lang = language;
    let cancelled = false;
    html = null;
    void highlightCode(code, lang).then((h) => {
      if (!cancelled) html = h;
    });
    return () => {
      cancelled = true;
    };
  });
</script>

{#if html}
  <div class="code" data-selectable>{@html html}</div>
{:else}
  <pre class="code plain" data-selectable><code>{source}</code></pre>
{/if}

<style>
  .code {
    /* Inline caps the height (24rem) and scrolls in place; the pane sets
       --artifact-max-height: none so the code fills the focused view and the
       pane body owns the scroll. */
    max-height: var(--artifact-max-height, 24rem);
    overflow: auto;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    cursor: text;
    /* `text` (not the app's `data-selectable` -> auto, which inherits the
       global `none`) so a mouse range is selectable + copyable. */
    -webkit-user-select: text;
    user-select: text;
  }
  .plain {
    margin: 0;
    padding: 0.75rem 0.875rem;
    font-family: var(--font-mono, ui-monospace, monospace);
    /* Inline stays compact; the pane sets --artifact-code-size larger. */
    font-size: var(--artifact-code-size, 0.75rem);
    line-height: 1.6;
    white-space: pre;
  }
  /* Shiki renders <pre class="shiki"><code>…; sit it on the frame, not Shiki's
     own background, and let the wrapper own the scroll. */
  :global(.code .shiki) {
    margin: 0;
    padding: 0.75rem 0.875rem;
    background: transparent !important;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--artifact-code-size, 0.75rem);
    line-height: 1.6;
    white-space: pre;
  }
</style>
