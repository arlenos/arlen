<script lang="ts">
  /// One message, and only what can be shown honestly.
  ///
  /// The HTML part is deliberately absent - see the app's `lib.rs`. The sentence
  /// that says so is a fact about the message, not an apology for a gap: a
  /// reader who is told nothing would take the text part for the whole message,
  /// which is exactly how a phishing mail whose two parts disagree gets read as
  /// the harmless one.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Mail, TriangleAlert } from "@lucide/svelte";
  import { t, locale } from "$lib/i18n/messages";

  /// A size written the way the reader writes one: `16 kB` in English, `16 kB`
  /// with a comma decimal in German, and the unit from the reader's locale rather
  /// than from a hardcoded table.
  ///
  /// Local rather than in the kit because `sdk/ui-kit` is arlen-ui's, and a
  /// second app that needs this is the moment to move it there - `formatDecimal`
  /// lives in the kit for exactly this reason and a byte size belongs beside it.
  const formatBytes = (n: number, loc: string) =>
    new Intl.NumberFormat(loc, {
      style: "unit",
      unit: "byte",
      unitDisplay: "narrow",
      notation: "compact",
      maximumFractionDigits: 1,
    }).format(n);

  /// When the message says it was sent, written the way the reader writes a date.
  ///
  /// The core hands over an RFC 3339 string, and the window was printing it
  /// verbatim: `2026-08-19T09:00:00Z` in front of a person, in every language.
  /// Every other app on this image formats through `Intl` off the shared locale -
  /// the calendar's own profile note makes it a house rule - and mail was the one
  /// surface still showing machine text.
  ///
  /// A DATE LINE IS WHATEVER THE SENDER WROTE, so this can fail, and when it does
  /// the string is shown exactly as it arrived rather than replaced by a guess or
  /// by nothing. The raw value stays on the element either way, so a person
  /// checking a suspicious message can still read the header as sent.
  /// The sentence for a calendar part, chosen by what the message claims it is.
  ///
  /// The method is a protocol token (`REQUEST`, `CANCEL`, ...) and the reader is
  /// not the protocol, so each known one has its own sentence in the catalogue.
  /// An unknown method is shown AS WRITTEN rather than dropped: a part marked
  /// something this app has never heard of is exactly when a person wants to see
  /// the word and decide for themselves.
  function invitationWords(method: string | null): string {
    // Written out rather than built with `ml.invitation.${method}`: the key gate
    // reads LITERAL keys, so a composed one is invisible to it and a rename
    // would take the sentence away with nothing failing.
    if (method === null) return $t("ml.invitation.unmarked");
    if (method === "request") return $t("ml.invitation.request");
    if (method === "cancel") return $t("ml.invitation.cancel");
    if (method === "reply") return $t("ml.invitation.reply");
    if (method === "publish") return $t("ml.invitation.publish");
    return $t("ml.invitation.other", { method });
  }

  function formatSent(raw: string, loc: string): string {
    const at = new Date(raw);
    if (Number.isNaN(at.getTime())) return raw;
    return new Intl.DateTimeFormat(loc, {
      dateStyle: "long",
      timeStyle: "short",
    }).format(at);
  }
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";

  type Message = {
    from: string | null;
    subject: string | null;
    date: string | null;
    text: string | null;
    has_html: boolean;
    only_in_text: string[];
    only_in_html: string[];
    refusal: string | null;
    to: string[];
    cc: string[];
    channels: string[];
    attachments: { name: string | null; media_type: string | null; bytes: number }[];
    invitation: { method: string | null; bytes: number } | null;
    path: string;
  };

  /// Whether there is a host to ask at all. In a browser there is none, and that
  /// is not a failure to report.
  const tauriAvailable = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  let message = $state<Message | null>(null);
  let failure = $state<string | null>(null);
  let launched = $state<string | null>(null);

  onMount(() => {
    if (!tauriAvailable) return;
    void (async () => {
      launched = await invoke<string | null>("launch_file").catch(() => null);
      if (!launched) return;
      try {
        message = await invoke<Message>("mail_read", { path: launched });
        failure = null;
      } catch (e) {
        failure = String(e);
      }
    })();
  });
</script>

<main class="page">
  <header class="bar">
    <Mail size={16} strokeWidth={2} />
    <h1>{$t("ml.app.title")}</h1>
    <span class="spacer"></span>
    <WindowButtons />
  </header>

  {#if failure}
    <p class="note bad" role="alert">{$t("ml.failed", { reason: failure })}</p>
  {:else if !message}
    <p class="note">{$t("ml.nothingOpen")}</p>
  {:else}
    <!-- The refusal comes FIRST and above the message, because it is a statement
         about whether anything below it can be believed. -->
    {#if message.refusal}
      <p class="note bad" role="alert">
        <TriangleAlert size={14} strokeWidth={2} aria-hidden="true" />
        {$t("ml.refused", { reason: message.refusal })}
      </p>
    {/if}

    <dl class="headers">
      <dt>{$t("ml.from")}</dt>
      <!-- The caveat sits ON the sender line rather than in a footnote: a
           display name is whatever the sender typed, and this is the field a
           reader trusts hardest. -->
      <dd>{message.from ?? "-"} <span class="quiet">({$t("ml.unsigned")})</span></dd>
      <dt>{$t("ml.subject")}</dt>
      <dd>{message.subject ?? "-"}</dd>
      <dt>{$t("ml.date")}</dt>
      <dd title={message.date ?? ""}>
        {message.date ? formatSent(message.date, $locale) : "-"}
      </dd>
      {#if message.to.length > 0}
        <dt>{$t("ml.to")}</dt>
        <dd>{message.to.join(", ")}</dd>
      {/if}
      {#if message.cc.length > 0}
        <dt>{$t("ml.cc")}</dt>
        <dd>{message.cc.join(", ")}</dd>
      {/if}
    </dl>

    {#if message.only_in_text.length > 0 || message.only_in_html.length > 0}
      <p class="note bad" role="status">
        {#if message.only_in_text.length > 0 && message.only_in_html.length > 0}
          {$t("ml.divergenceBoth", {
            text: message.only_in_text.join(", "),
            html: message.only_in_html.join(", "),
          })}
        {:else if message.only_in_text.length > 0}
          {$t("ml.divergenceText", { text: message.only_in_text.join(", ") })}
        {:else}
          {$t("ml.divergenceHtml", { html: message.only_in_html.join(", ") })}
        {/if}
      </p>
    {/if}

    {#if message.channels.length > 0}
      <p class="note bad" role="status">
        {$t("ml.channels", { list: message.channels.join(", ") })}
      </p>
    {/if}

    {#if message.invitation}
      <p class="note">{invitationWords(message.invitation.method)}</p>
    {/if}

    {#if message.attachments.length > 0}
      <p class="note">{$t("ml.carries", { count: message.attachments.length })}</p>
      <ul class="carried">
        {#each message.attachments as file, i (i)}
          <li>
            {$t("ml.attachment", {
              name: file.name ?? $t("ml.unnamedAttachment"),
              type: file.media_type ?? "?",
              size: formatBytes(file.bytes, $locale),
            })}
          </li>
        {/each}
      </ul>
    {/if}

    {#if message.text}
      <pre class="body">{message.text}</pre>
    {:else}
      <p class="note">{$t("ml.noText")}</p>
    {/if}

    {#if message.has_html}
      <p class="note">{$t("ml.htmlNotShown")}</p>
    {/if}
  {/if}
</main>

<style>
  .page {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #e6e8ee);
    font-family: "Inter Variable", system-ui, sans-serif;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--color-border-default, #2a2a2a);
  }
  .bar h1 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .note {
    margin: 12px 14px 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .note.bad {
    color: var(--color-fg-warning, #eab308);
  }
  .headers {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 14px;
    font-size: 13px;
  }
  .headers dt {
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .headers dd {
    margin: 0;
    /* The subject and the sender are the two fields most likely to be long and
       hostile; neither may push the layout sideways. */
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .quiet {
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .carried {
    margin: 0.25rem 0 0.75rem;
    padding-left: 1.1rem;
  }
  .carried li {
    opacity: 0.85;
  }
  .body {
    margin: 8px 14px 14px;
    padding: 12px;
    overflow: auto;
    font-family: ui-monospace, monospace;
    font-size: 13px;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    background: var(--color-bg-card, #171717);
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: 8px;
  }
</style>
