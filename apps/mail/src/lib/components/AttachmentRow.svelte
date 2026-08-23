<script lang="ts">
  /// Attachments as a chip row: name, type, size - NAMED AND MEASURED, NEVER
  /// OPENED (the core's rule), so the chips are deliberately not buttons. The
  /// carries-count sentence above them keeps saying so. A calendar part gets
  /// the calendar icon; its own sentence lives in the notices.
  import { Paperclip, CalendarDays } from "@lucide/svelte";
  import { t, locale } from "$lib/i18n/messages";
  import { formatBytes } from "$lib/wording";
  import type { Message } from "$lib/stores/mailbox";

  let { message }: { message: Message } = $props();
</script>

{#if message.attachments.length > 0}
  <div class="attachments">
    <p class="carries">{$t("ml.carries", { count: message.attachments.length })}</p>
    <div class="chips">
      {#each message.attachments as file, i (i)}
        <span class="chip">
          {#if file.media_type === "text/calendar"}
            <CalendarDays size={13} strokeWidth={1.75} aria-hidden="true" />
          {:else}
            <Paperclip size={13} strokeWidth={1.75} aria-hidden="true" />
          {/if}
          <span class="chip-name">{file.name ?? $t("ml.unnamedAttachment")}</span>
          <span class="chip-meta">{formatBytes(file.bytes, $locale)}</span>
        </span>
      {/each}
    </div>
  </div>
{/if}

<style>
  .attachments {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .carries {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: var(--radius-chip, 999px);
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
  }
  .chip :global(svg) {
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .chip-name {
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip-meta {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
