<script lang="ts">
  /// One short thing in a row of them: a tag, a facet, a file in the context,
  /// a capability's reach. Twelve apps drew their own, and the kit itself had
  /// two (the chip list's and the scope chip's) that disagreed on height,
  /// border and fill. This is the one, in the scope chip's flat register: a
  /// quiet fill, no border, the chip radius.
  ///
  /// `onremove` adds a remove control named "Remove <label>" in the reader's
  /// language. `onclick` makes the label itself a button (open the file, jump
  /// to the facet). `lead` is a slot before the label for an icon or a mark.
  /// A long label ellipses; the chip never grows past its row.
  import type { Snippet } from "svelte";
  import { X } from "@lucide/svelte";
  import { kt } from "../../../i18n/messages.kit";

  let {
    label,
    id,
    lead,
    onclick,
    onremove,
    class: className,
  }: {
    label: string;
    id?: string;
    /// An icon or mark before the label.
    lead?: Snippet;
    /// Makes the label a button.
    onclick?: (e: MouseEvent) => void;
    /// Adds a remove control; the caller owns any confirm step.
    onremove?: () => void;
    class?: string;
  } = $props();
</script>

<span class="chip {className ?? ''}" class:has-x={!!onremove} {id}>
  {#if onclick}
    <button type="button" class="chip-body" {onclick}>
      {#if lead}<span class="chip-lead" aria-hidden="true">{@render lead()}</span>{/if}
      <span class="chip-label">{label}</span>
    </button>
  {:else}
    <span class="chip-body">
      {#if lead}<span class="chip-lead" aria-hidden="true">{@render lead()}</span>{/if}
      <span class="chip-label">{label}</span>
    </span>
  {/if}
  {#if onremove}
    <button type="button" class="chip-x" aria-label={$kt("k.chip.remove", { label })} onclick={onremove}>
      <X size={12} strokeWidth={2.5} />
    </button>
  {/if}
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    max-width: 100%;
    height: 1.5rem;
    padding: 0 0.5rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: var(--text-xs);
    line-height: 1;
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .chip.has-x {
    padding-inline-end: 0.1875rem;
  }
  .chip-body {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    font: inherit;
    color: inherit;
  }
  button.chip-body:hover {
    color: var(--foreground);
  }
  .chip-lead {
    display: inline-flex;
    flex-shrink: 0;
  }
  /* The label clips for its ellipsis, so it needs its own line box: at the
     chip's line-height of 1 a descender ("g", "p") was cut off. */
  .chip-label {
    min-width: 0;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1rem;
    height: 1rem;
    flex-shrink: 0;
    padding: 0;
    border: none;
    border-radius: max(0px, calc(var(--radius-chip, 4px) - 2px));
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .chip-x:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
  }
</style>
