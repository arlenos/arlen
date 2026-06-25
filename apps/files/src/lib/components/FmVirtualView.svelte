<script lang="ts">
  /// The chrome shared by the FM's virtual views (Recent, Trash): a header with
  /// the title, optional header actions and a close button, then the list body
  /// with its loading and empty states. Each view supplies only its own rows;
  /// this owns the frame so the virtual views read as one surface.
  import { X } from "lucide-svelte";
  import type { Snippet } from "svelte";

  type Props = {
    /// The view's name, shown in the header and the close label.
    title: string;
    /// Close the view.
    onClose: () => void;
    /// The list is still loading (the store is null).
    loading?: boolean;
    /// The list loaded but is empty.
    empty?: boolean;
    /// The line shown when `empty`.
    emptyLabel?: string;
    /// Optional header controls, left of the close button (e.g. Empty Trash).
    actions?: Snippet;
    /// The list body, rendered when neither loading nor empty.
    children: Snippet;
  };

  let {
    title,
    onClose,
    loading = false,
    empty = false,
    emptyLabel = "Nothing here",
    actions,
    children,
  }: Props = $props();
</script>

<div class="vv">
  <div class="vv-head">
    <span class="vv-title">{title}</span>
    {#if actions}
      <div class="vv-actions">{@render actions()}</div>
    {/if}
    <button
      class="vv-close"
      aria-label={`Close ${title.toLowerCase()}`}
      onclick={onClose}
    >
      <X size={14} strokeWidth={2} />
    </button>
  </div>

  {#if loading}
    <div class="vv-empty-state">Loading…</div>
  {:else if empty}
    <div class="vv-empty-state">{emptyLabel}</div>
  {:else}
    {@render children()}
  {/if}
</div>

<style>
  .vv {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }
  .vv-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .vv-title {
    flex: 1;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .vv-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .vv-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .vv-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .vv-empty-state {
    margin: auto;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
