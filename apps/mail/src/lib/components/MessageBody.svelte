<script lang="ts">
  /// The trust-and-content half of one message: the notices in tone order
  /// (error before caution before neutral - the refusal is a statement about
  /// whether anything below it can be believed), the attachments, the body in
  /// reading typography. Shared between the single reading surface and a
  /// conversation's blocks, so a message says the same things wherever it is
  /// read.
  import { t } from "$lib/i18n/messages";
  import { invitationWords } from "$lib/wording";
  import type { Message } from "$lib/stores/mailbox";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
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

{#if message.refusal || divergence || message.channels.length > 0 || sealedText || message.has_html || message.invitation}
  <div class="notices">
    {#if message.refusal}
      <Notice tone="error" text={$t("ml.refused", { reason: message.refusal })} />
    {/if}
    {#if divergence}
      <Notice tone="caution" text={divergence} />
    {/if}
    {#if message.channels.length > 0}
      <Notice tone="caution" text={$t("ml.channels", { list: message.channels.join(", ") })} />
    {/if}
    {#if sealedText}
      <Notice tone="neutral" text={sealedText} />
    {/if}
    {#if message.invitation}
      <Notice tone="neutral" text={invitationWords(message.invitation.method, $t)} />
    {/if}
    {#if message.has_html}
      <Notice tone="neutral" text={$t("ml.htmlNotShown")} />
    {/if}
  </div>
{/if}

<AttachmentRow {message} />

{#if message.text}
  <div class="body">{message.text}</div>
{:else if !sealedText}
  <p class="no-text">{$t("ml.noText")}</p>
{/if}

<style>
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
