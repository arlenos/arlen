<script lang="ts">
  /// The no-messages surface, one designed variant per capability state. No
  /// decorative icon; the words and the starters carry it. The assistant's
  /// no-memory limitation is stated here, once, before anyone relies on it
  /// (design-system.md 6.6), and nowhere per row.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { t } from "$lib/i18n/messages";
  import { openAiSettings, settingsOpenFailed } from "$lib/transparency";

  let {
    variant,
    onstarter,
    onretry,
  }: {
    variant: "ready" | "off" | "unreachable";
    /// Fills the composer with a starter prompt (editable before sending).
    onstarter: (text: string) => void;
    onretry: () => void;
  } = $props();

  // Starter prompts grounded in what the assistant can actually answer.
  const STARTERS = $derived([
    $t("h.empty.starter.yesterday"),
    $t("h.empty.starter.project"),
    $t("h.empty.starter.downloads"),
  ]);
</script>

<div class="empty">
  {#if variant === "ready"}
    <p class="title">{$t("h.empty.ready.title")}</p>
    <p class="sub">{$t("h.empty.ready.sub")}</p>
    <p class="limit">{$t("h.empty.ready.limit")}</p>
    <div class="starters">
      {#each STARTERS as s (s)}
        <button type="button" class="starter" onclick={() => onstarter(s)}>{s}</button>
      {/each}
    </div>
  {:else if variant === "off"}
    <p class="title">{$t("h.empty.off.title")}</p>
    <p class="sub">{$t("h.empty.off.sub")}</p>
    <!-- The sentence names Settings, so the surface goes there: the master
         switch lives in the Settings app, and this is the one full-screen
         state a person meets with nothing else to press. -->
    <Button variant="default" size="sm" class="mt-2" onclick={openAiSettings}>{$t("h.offswitch.openSettings")}</Button>
    {#if $settingsOpenFailed}
      <Notice tone="error" class="mt-2" text={$t("h.settings.cannotOpen")} />
    {/if}
  {:else}
    <p class="title">{$t("h.empty.unreachable.title")}</p>
    <p class="sub">{$t("h.empty.unreachable.sub")}</p>
    <Button variant="outline" size="sm" class="mt-2" onclick={onretry}>{$t("h.tryAgain")}</Button>
  {/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    max-width: 28rem;
    margin-inline: auto;
    text-align: center;
    gap: 0.5rem;
  }
  .title {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--foreground);
  }
  .sub {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .limit {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .starters {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 0.5rem;
    margin-top: 1rem;
  }
  .starter {
    display: inline-flex;
    align-items: center;
    min-height: var(--height-control, 28px);
    padding: 0 0.75rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-button);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font-size: var(--text-sm);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .starter:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    color: var(--foreground);
  }
</style>
