<script lang="ts">
  /// The reading surface: head, then the notices in one language (error before
  /// caution before neutral - the refusal is a statement about whether anything
  /// below it can be believed, so it comes first), attachments, then the body
  /// in reading typography. The HTML part stays deliberately absent (the app's
  /// lib.rs, EFAIL); the sentence saying so is a fact about the message, not an
  /// apology for a gap.
  import { t } from "$lib/i18n/messages";
  import { invitationWords } from "$lib/wording";
  import type { Message } from "$lib/stores/mailbox";
  import MessageHeader from "./MessageHeader.svelte";
  import MessageNotice from "./MessageNotice.svelte";
  import AttachmentRow from "./AttachmentRow.svelte";

  let { message }: { message: Message } = $props();

  const divergence = $derived.by(() => {
    const m = message;
    if (m.only_in_text.length > 0 && m.only_in_html.length > 0)
      return $t("ml.divergenceBoth", { text: m.only_in_text.join(", "), html: m.only_in_html.join(", ") });
    if (m.only_in_text.length > 0) return $t("ml.divergenceText", { text: m.only_in_text.join(", ") });
    if (m.only_in_html.length > 0) return $t("ml.divergenceHtml", { html: m.only_in_html.join(", ") });
    return null;
  });

  const sealedText = $derived(
    message.sealed === "pgp"
      ? $t("ml.sealed.pgp")
      : message.sealed === "smime"
        ? $t("ml.sealed.smime")
        : message.sealed
          ? $t("ml.sealed.unknown")
          : null,
  );
</script>

<article class="message">
  <MessageHeader {message} />

  {#if message.refusal || divergence || message.channels.length > 0 || sealedText || message.has_html || message.invitation}
    <div class="notices">
      {#if message.refusal}
        <MessageNotice tone="error" text={$t("ml.refused", { reason: message.refusal })} />
      {/if}
      {#if divergence}
        <MessageNotice tone="caution" text={divergence} />
      {/if}
      {#if message.channels.length > 0}
        <MessageNotice tone="caution" text={$t("ml.channels", { list: message.channels.join(", ") })} />
      {/if}
      {#if sealedText}
        <MessageNotice tone="neutral" text={sealedText} />
      {/if}
      {#if message.invitation}
        <MessageNotice tone="neutral" text={invitationWords(message.invitation.method, $t)} />
      {/if}
      {#if message.has_html}
        <MessageNotice tone="neutral" text={$t("ml.htmlNotShown")} />
      {/if}
    </div>
  {/if}

  <AttachmentRow {message} />

  {#if message.text}
    <div class="body">{message.text}</div>
  {:else if !sealedText}
    <p class="no-text">{$t("ml.noText")}</p>
  {/if}
</article>

<style>
  .message {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    width: 100%;
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .notices {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .body {
    font-size: var(--text-sm, 13px);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: color-mix(in srgb, var(--color-fg-primary) 88%, transparent);
  }
  .no-text {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
