<script lang="ts">
  /// The message head: subject as the title, then the sender line with a
  /// monogram avatar and the not-verified caveat ON the line - the display
  /// name is whatever the sender typed and this is the field a reader trusts
  /// hardest. Recipients stay one quiet line each; the date is written in the
  /// reader's language (a malformed header shows verbatim, wording.ts).
  import { t, locale } from "$lib/i18n/messages";
  import { displayName, formatSent } from "$lib/wording";
  import type { Message } from "$lib/stores/mailbox";

  let { message }: { message: Message } = $props();

  const name = $derived(message.from ? displayName(message.from) : "?");
  const letter = $derived((name.trim()[0] ?? "?").toUpperCase());
</script>

<header class="head">
  <h2 class="subject">{message.subject ?? "-"}</h2>
  <div class="from-line">
    <span class="avatar" aria-hidden="true">{letter}</span>
    <div class="who">
      <p class="from">
        {message.from ?? "-"}
        <span class="caveat">({$t("ml.unsigned")})</span>
      </p>
      {#if message.to.length > 0}
        <p class="rcpt">{$t("ml.to")}: {message.to.join(", ")}</p>
      {/if}
      {#if message.cc.length > 0}
        <p class="rcpt">{$t("ml.cc")}: {message.cc.join(", ")}</p>
      {/if}
    </div>
    {#if message.date}
      <span class="when">{formatSent(message.date, $locale)}</span>
    {/if}
  </div>
</header>

<style>
  .head {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .subject {
    margin: 0;
    font-size: var(--text-xl, 19px);
    font-weight: 600;
    line-height: 1.3;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .from-line {
    display: flex;
    gap: 0.7rem;
    align-items: flex-start;
  }
  .avatar {
    display: flex;
    flex-shrink: 0;
    width: 2.25rem;
    height: 2.25rem;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-sm, 13px);
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .from {
    margin: 0;
    font-size: var(--text-sm, 13px);
    font-weight: 500;
    /* Hostile senders may be long; nothing pushes the layout sideways. */
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .caveat {
    font-weight: 400;
    color: color-mix(in srgb, var(--color-fg-primary) 48%, transparent);
  }
  .rcpt {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .when {
    flex-shrink: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
