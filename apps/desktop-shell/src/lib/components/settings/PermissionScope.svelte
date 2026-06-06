<script lang="ts">
  /// Reusable permission scope display component.
  /// Shows a category name with a list of items (patterns, booleans, domains).

  import { Check, X } from "lucide-svelte";

  interface Props {
    title: string;
    items?: string[];
    booleans?: { label: string; value: boolean }[];
  }

  let { title, items, booleans }: Props = $props();
</script>

<div class="scope">
  <div class="scope-title">{title}</div>
  {#if booleans}
    <div class="scope-list">
      {#each booleans as b}
        <div class="scope-bool">
          {#if b.value}
            <Check size={12} strokeWidth={2} class="scope-yes" />
          {:else}
            <X size={12} strokeWidth={2} class="scope-no" />
          {/if}
          <span>{b.label}</span>
        </div>
      {/each}
    </div>
  {/if}
  {#if items && items.length > 0}
    <div class="scope-list">
      {#each items as item}
        <span class="scope-chip">{item}</span>
      {/each}
    </div>
  {:else if !booleans}
    <span class="scope-none">None</span>
  {/if}
</div>

<style>
  .scope { margin-bottom: 12px; }
  .scope-title { font-size: 0.6875rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; opacity: 0.5; margin-bottom: 4px; }
  .scope-list { display: flex; flex-wrap: wrap; gap: 4px; }
  .scope-chip { font-size: 0.75rem; padding: 2px 8px; background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); border-radius: var(--radius-sm, 4px); }
  .scope-bool { display: flex; align-items: center; gap: 4px; font-size: 0.75rem; padding: 2px 0; }
  :global(.scope-yes) { color: var(--color-success); }
  :global(.scope-no) { color: var(--color-fg-disabled); }
  .scope-none { font-size: 0.75rem; opacity: 0.4; }
</style>
