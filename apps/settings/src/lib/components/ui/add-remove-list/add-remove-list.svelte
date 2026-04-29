<script lang="ts" generics="T">
  /// Generic editable list for settings pages.
  ///
  /// Renders one row per item via the `itemSnippet`, plus a footer
  /// "Add" button. The remove button is rendered by this component
  /// (consistent X-icon, consistent styling) so consumers only own
  /// the item content.
  ///
  /// Examples:
  /// - Window Rules (each item = matcher + action pair)
  /// - Watch Directories (each item = path string)
  /// - Default suppressed apps (each item = app id)
  ///
  /// The Add trigger is opaque: the consumer wires `onadd` to whatever
  /// dialog or picker their data type needs. We deliberately don't
  /// bake a picker into this list — different consumers need different
  /// pickers (DirectoryPicker, AppPicker, plain text input, etc.).

  import type { Snippet } from "svelte";
  import { X, Plus } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button";

  let {
    items,
    itemSnippet,
    onremove,
    onadd,
    addLabel = "Add",
    emptyMessage = "No items yet.",
    disabled = false,
  }: {
    items: T[];
    itemSnippet: Snippet<[{ item: T; index: number }]>;
    onremove: (index: number) => void;
    onadd: () => void;
    addLabel?: string;
    emptyMessage?: string;
    disabled?: boolean;
  } = $props();
</script>

<div class="list">
  {#if items.length === 0}
    <div class="empty">{emptyMessage}</div>
  {:else}
    <div class="items">
      {#each items as item, i (i)}
        <div class="item">
          <div class="content">
            {@render itemSnippet({ item, index: i })}
          </div>
          <button
            type="button"
            class="remove"
            aria-label="Remove"
            {disabled}
            onclick={() => onremove(i)}
          >
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      {/each}
    </div>
  {/if}
  <div class="footer">
    <Button variant="outline" size="sm" {disabled} onclick={onadd}>
      <Plus size={14} />
      {addLabel}
    </Button>
  </div>
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .empty {
    padding: 0.75rem 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    border: 1px dashed
      color-mix(in srgb, var(--foreground) 14%, transparent);
    border-radius: var(--radius-sm);
    text-align: center;
  }

  .items {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.375rem 0.5rem 0.375rem 0.75rem;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    transition: background 0.1s ease;
  }

  .item:hover {
    background: color-mix(in srgb, var(--foreground) 7%, transparent);
  }

  .content {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    color: var(--foreground);
  }

  .remove {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    border: 1px solid transparent;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
    transition:
      background 0.1s ease,
      color 0.1s ease;
  }

  .remove:hover:not(:disabled) {
    background: color-mix(in srgb, var(--destructive) 12%, transparent);
    color: var(--destructive);
  }

  .remove:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .footer {
    display: flex;
    justify-content: flex-start;
  }
</style>
