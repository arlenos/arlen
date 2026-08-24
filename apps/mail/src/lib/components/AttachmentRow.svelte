<script lang="ts">
  /// Attachments as a chip row: name, type, size - and a press SAVES the file
  /// out, it never opens or previews it here (the core's named-and-measured
  /// rule; previewing is one of §3's backchannels). The save command is the
  /// intended `mail_save_attachment(path, index)` seam; until it exists the
  /// press answers with an honest refusal line rather than pretending.
  import { Paperclip, CalendarDays, Download } from "@lucide/svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t, locale } from "$lib/i18n/messages";
  import { formatBytes } from "$lib/wording";
  import type { Message } from "$lib/stores/mailbox";

  let { message }: { message: Message } = $props();

  /// The outcome of the last save press, one line under the chips.
  let outcome = $state<{ ok: boolean; text: string } | null>(null);

  /// What the command says went wrong, as a word this file turns into a sentence.
  type SaveProblem =
    | { problem: "unreadable"; why: string }
    | { problem: "not-a-message" }
    | { problem: "no-such-attachment" }
    | { problem: "no-folder" }
    | { problem: "not-written"; why: string };

  function refusal(e: unknown): string {
    // A TAGGED word, not a stringified object. `String(e)` on a typed refusal
    // renders `[object Object]`, which is a sentence about nothing.
    const p = e as SaveProblem | null;
    switch (p?.problem) {
      case "unreadable":
        return $t("ml.attach.failed.unreadable", { why: p.why });
      case "not-a-message":
        return $t("ml.attach.failed.notAMessage");
      case "no-such-attachment":
        return $t("ml.attach.failed.noSuchAttachment");
      case "no-folder":
        return $t("ml.attach.failed.noFolder");
      case "not-written":
        return $t("ml.attach.failed.notWritten", { why: p.why });
      default:
        // Not one of ours: no host at all, or a transport that failed before the
        // command answered. Its own words are the only detail there is - and they
        // are developer English, or `[object Object]` for a tag added later, so
        // they go to the console rather than into a translated sentence.
        console.warn("mail: unrecognised attachment refusal", e);
        return $t("ml.attach.failed.other");
    }
  }

  async function save(index: number): Promise<void> {
    try {
      const path = await invoke<string>("mail_save_attachment", { path: message.path, index });
      outcome = { ok: true, text: $t("ml.attach.saved", { path }) };
    } catch (e) {
      outcome = { ok: false, text: refusal(e) };
    }
  }
</script>

{#if message.attachments.length > 0}
  <div class="attachments">
    <p class="carries">{$t("ml.carries", { count: message.attachments.length })}</p>
    <div class="chips">
      {#each message.attachments as file, i (i)}
        {@const name = file.name ?? $t("ml.unnamedAttachment")}
        <button type="button" class="chip" aria-label={$t("ml.attach.save", { name })} onclick={() => save(i)}>
          {#if file.media_type === "text/calendar"}
            <CalendarDays size={13} strokeWidth={1.75} aria-hidden="true" />
          {:else}
            <Paperclip size={13} strokeWidth={1.75} aria-hidden="true" />
          {/if}
          <span class="chip-name">{name}</span>
          <span class="chip-meta">{formatBytes(file.bytes, $locale)}</span>
          <Download size={12} strokeWidth={1.75} class="chip-save" aria-hidden="true" />
        </button>
      {/each}
    </div>
    {#if outcome}
      <p class="outcome" class:bad={!outcome.ok} role="status">{outcome.text}</p>
    {/if}
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
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
    cursor: pointer;
  }
  .chip:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .chip:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .chip :global(svg) {
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .chip :global(.chip-save) {
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
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
  .outcome {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .outcome.bad {
    color: var(--color-warning, #eab308);
  }
</style>
