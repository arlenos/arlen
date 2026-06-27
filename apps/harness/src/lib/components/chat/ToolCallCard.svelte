<script lang="ts">
  /// One tool call the assistant made while answering, as a full-width
  /// collapsible: a plain summary line on the surface, the raw tool name,
  /// arguments and result behind the chevron. Transparency-first, two layers.
  import { Check, ChevronDown, LoaderCircle, X } from "@lucide/svelte";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import type { ToolCall } from "$lib/stores/conversation";
  import { toolLabel } from "$lib/display";

  let { call }: { call: ToolCall } = $props();

  const id = $derived(`${call.server}/${call.tool}`);
  // The status label voices the glyph for assistive tech; quiet for the
  // expected case, the error colour is the only one that draws the eye.
  const statusLabel = $derived(
    call.status === "failed"
      ? "Failed"
      : call.status === "running"
        ? "Running"
        : call.status === "done"
          ? "Done"
          : null,
  );
</script>

<Collapsible>
  <CollapsibleTrigger class="tc-summary">
    <span class="tc-left">
      {#if statusLabel}
        <span class="tc-status" data-status={call.status} aria-label={statusLabel}>
          {#if call.status === "failed"}
            <X size={13} strokeWidth={2.25} />
          {:else if call.status === "running"}
            <LoaderCircle size={13} strokeWidth={2.25} />
          {:else}
            <Check size={13} strokeWidth={2.25} />
          {/if}
        </span>
      {/if}
      <span class="tc-label">{toolLabel(id)}</span>
    </span>
    <span class="tc-chevron"><ChevronDown size={14} strokeWidth={2} /></span>
  </CollapsibleTrigger>
  <CollapsibleContent>
    <div class="tc-detail">
      <div class="tc-section">
        <span class="tc-key">Tool</span>
        <code class="tc-name">{id}</code>
      </div>
      {#if call.arguments}
        <div class="tc-section">
          <span class="tc-key">Asked for</span>
          <pre>{call.arguments}</pre>
        </div>
      {/if}
      {#if call.result}
        <div class="tc-section">
          <span class="tc-key">Result</span>
          <pre>{call.result}</pre>
        </div>
      {/if}
    </div>
  </CollapsibleContent>
</Collapsible>

<style>
  :global(.tc-summary) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: var(--height-control, 28px);
    padding: 0 var(--space-card, 1rem);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    transition: color var(--duration-fast) var(--ease-out);
  }
  :global(.tc-summary:hover) {
    color: var(--foreground);
  }
  :global(.tc-summary[data-state="open"]) {
    border-bottom-left-radius: 0;
    border-bottom-right-radius: 0;
  }
  :global(.tc-summary[data-state="open"]) .tc-chevron {
    transform: rotate(180deg);
  }
  .tc-left {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  /* The status glyph stays as quiet as the summary text for the expected
     done/running cases; only a failure takes the error colour. */
  .tc-status {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .tc-status[data-status="failed"] {
    color: var(--color-error);
  }
  .tc-status[data-status="running"] {
    animation: tc-spin 1s linear infinite;
  }
  @keyframes tc-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .tc-label {
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tc-chevron {
    display: inline-flex;
    flex-shrink: 0;
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .tc-detail {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem var(--space-card, 1rem);
    border: 1px solid var(--color-border);
    border-top: none;
    border-bottom-left-radius: var(--radius-input);
    border-bottom-right-radius: var(--radius-input);
  }
  .tc-section {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 0;
  }
  .tc-key {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .tc-name {
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }
  pre {
    margin: 0;
    padding: 0.5rem 0.75rem;
    background: var(--color-bg-app);
    border-radius: var(--radius-chip);
    font-size: 0.75rem;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-x: auto;
  }
</style>
