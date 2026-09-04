<script lang="ts">
  /// The quiet status line under the composer: capability and posture as one
  /// plain sentence, anchored by the capability glyph. The model in use is
  /// visible in the picker above the composer and the no-memory limitation is
  /// stated once where a conversation begins, so the line carries no tooltip.
  /// This is the in-body capability strip; nothing about it lives in the header.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import type { Capability } from "$lib/capability";
  import { openAiSettings, settingsOpenFailed } from "$lib/transparency";
  import { statusSentence } from "$lib/display";
  import { t } from "$lib/i18n/messages";

  let {
    capability,
    loaded,
    onretry,
  }: {
    /// The capability read; `null` after a failed read.
    capability: Capability | null;
    /// False until the first read settles, so nothing flashes.
    loaded: boolean;
    onretry: () => void;
  } = $props();
</script>

{#if loaded}
  <div class="status">
    {#if capability}
      <p class="line">
        <span class="dot" class:off={!capability.enabled} aria-hidden="true"></span>
        {statusSentence(capability, $t)}
      </p>
      {#if !capability.enabled}
        <Button variant="outline" size="sm" onclick={openAiSettings}>{$t("h.offswitch.openSettings")}</Button>
      {/if}
    {:else}
      <p class="line">{$t("h.capability.unreachable")}</p>
      <Button variant="outline" size="sm" onclick={onretry}>{$t("h.tryAgain")}</Button>
    {/if}
  </div>
  {#if $settingsOpenFailed}
    <div class="status"><Notice tone="error" text={$t("h.settings.cannotOpen")} /></div>
  {/if}
{/if}

<style>
  .status {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
    margin-top: 0.5rem;
    min-width: 0;
  }
  .line {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The capability mark is a status dot of the house family: 6px on the
     chip radius, success when the assistant is on, quiet when it is off. */
  .dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    margin-inline-end: 0.4rem;
    vertical-align: middle;
    border-radius: var(--radius-chip, 4px);
    background: var(--color-success);
  }
  .dot.off {
    background: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
</style>
