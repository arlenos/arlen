<script lang="ts">
  /// The "Arlen drafted this filter" banner, shown above the facet chips when a
  /// natural-language ask produced the current filter. It names the question and
  /// what was read (the transparency line: the reads are shown, the audit is the
  /// guarantee) so the drafted facets below read as a verifiable suggestion, not
  /// a black box. Dismiss reverts the draft. Pull, never push.
  import { Sparkles } from "lucide-svelte";
  import { askDraft } from "$lib/stores/ask";
  import { t } from "$lib/i18n/messages";

  let {
    scope,
    ondismiss,
  }: {
    /// The folder the ask was scoped to, shown in the reads line.
    scope: string;
    /// Revert the draft (clear the facets + the banner, return to the folder).
    ondismiss?: () => void;
  } = $props();

  const shortScope = $derived(scope.replace(/^\/home\/[^/]+/, "~"));
</script>

{#if $askDraft}
  <div class="ask-banner">
    <div class="ask-line">
      <Sparkles size={13} strokeWidth={2} class="ask-spark" />
      <span class="ask-label">{$t("f.ask.draftedFrom")}</span>
      <span class="ask-query">{$askDraft.query}</span>
      <span class="ask-spacer"></span>
      <button class="ask-dismiss" onclick={() => ondismiss?.()}>{$t("f.dismiss")}</button>
    </div>
    <div class="ask-reads">
      {$t("f.ask.readsFiles", { count: $askDraft.reads.files, scope: shortScope })}
      {#if $askDraft.reads.tags > 0}
        , {$t("f.ask.readsTags", { count: $askDraft.reads.tags })}
      {/if}
    </div>
  </div>
{/if}

<style>
  .ask-banner {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .ask-line {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.8125rem;
  }
  .ask-line :global(.ask-spark) {
    flex-shrink: 0;
    color: var(--color-accent);
  }
  .ask-label {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ask-query {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--foreground);
    font-style: italic;
  }
  .ask-spacer {
    flex: 1;
  }
  .ask-dismiss {
    flex-shrink: 0;
    height: 22px;
    padding: 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-input);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.75rem;
    font-weight: 500;
  }
  .ask-dismiss:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .ask-reads {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
