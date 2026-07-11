<script lang="ts">
  /// Attached-context chips inside the composer: the files grounding the
  /// next turn, each removable. The "grounded in your own data" gesture made
  /// visible.
  import { Paperclip, X } from "@lucide/svelte";
  import type { MentionContent } from "$lib/stores/conversation";

  let {
    attached,
    onremove,
  }: {
    attached: MentionContent[];
    onremove: (path: string) => void;
  } = $props();
</script>

{#if attached.length > 0}
  <div class="chips">
    {#each attached as m (m.path)}
      <span class="chip" title={m.path}>
        <Paperclip size={12} strokeWidth={2} />
        <span class="chip-name">{m.name}{m.truncated ? " (shortened)" : ""}</span>
        <button
          type="button"
          class="chip-x"
          aria-label={`Remove ${m.name}`}
          onclick={() => onremove(m.path)}
        >
          <X size={12} strokeWidth={2.5} />
        </button>
      </span>
    {/each}
  </div>
{/if}

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.75rem 1rem 0;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    height: var(--height-control-compact, 24px);
    padding: 0 0.25rem 0 0.5rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-button);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    max-width: 16rem;
  }
  .chip :global(svg) {
    flex-shrink: 0;
  }
  .chip-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    border-radius: var(--radius-chip);
  }
  .chip-x:hover {
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
