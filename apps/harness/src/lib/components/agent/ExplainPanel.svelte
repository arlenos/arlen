<script lang="ts">
  /// System Explanation Mode (Foundation §5.8): an on-demand plain-language
  /// summary of what the computer is doing now. Rendering only; the page
  /// owns the explain call.
  import { t } from "$lib/i18n/messages";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { renderMarkdown } from "$lib/markdown";
  import { externalLinks } from "$lib/externalLinks";

  let {
    explanation,
    error,
    busy,
    aiOff,
    onexplain,
  }: {
    explanation: string | null;
    error: string | null;
    busy: boolean;
    /// The AI master switch is off, so nothing can be explained.
    aiOff: boolean;
    onexplain: () => void;
  } = $props();
</script>

<div class="explain">
  {#if aiOff}
    <p class="hint">{$t("h.explain.off")}</p>
  {:else}
    <p class="hint">{$t("h.explain.prompt")}</p>
  {/if}
  <Button variant="default" size="sm" disabled={busy || aiOff} onclick={onexplain}>
    {busy ? $t("h.explain.working") : $t("h.explain.button")}
  </Button>
  {#if error}
    <p class="error">{$t("h.explain.failed")}</p>
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
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .text {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: var(--foreground);
  }
  .error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--color-error);
  }
</style>
