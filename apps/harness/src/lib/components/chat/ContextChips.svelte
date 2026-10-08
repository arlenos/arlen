<script lang="ts">
  /// Attached-context chips inside the composer: the files grounding the
  /// next turn, each removable. The "grounded in your own data" gesture made
  /// visible. The chip shows the file's name; its full path is the identity
  /// reveal design-system.md 6.6 allows, through the kit tooltip and carrying
  /// the path and nothing else.
  import { Paperclip } from "@lucide/svelte";
  import { Chip } from "@arlen/ui-kit/components/ui/chip";
  import { t } from "$lib/i18n/messages";
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
      <Chip
        label={m.truncated ? $t("h.composer.shortened", { name: m.name }) : m.name}
        tooltip={m.path}
        removeLabel={$t("h.composer.removeFile", { name: m.name })}
        onremove={() => onremove(m.path)}
      >
        {#snippet lead()}<Paperclip size={12} strokeWidth={2} />{/snippet}
      </Chip>
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
  .chips :global(.chip) {
    max-width: 16rem;
  }
</style>
