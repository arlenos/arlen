<script lang="ts">
  /// Home: the sidebar carries the meeting list now, so this surface is the
  /// quiet start point - one action, one hint. Rows and history live in the
  /// rail, always visible, never one Back away.
  import { goto } from "$app/navigation";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t, dir } from "$lib/i18n/messages";
  import { meetings, meetingsMocked, meetingsUnavailable } from "$lib/stores/meeting";
</script>

<div class="home" dir={$dir}>
  <div class="home-center">
    {#if $meetingsMocked}
      <p class="sample">{$t("mt.sample.list")}</p>
    {:else if $meetingsUnavailable}
      <p class="sample">{$t("mt.unavailable")}</p>
    {/if}
    <!-- Same three states. When the read failed the sentence above already says
         so, and adding "No meetings yet" underneath would answer a question we
         just said we could not answer. -->
    {#if !$meetingsUnavailable}
      <p class="hint">{$meetings.length === 0 ? $t("mt.empty") : $t("mt.pickHint")}</p>
    {/if}
    <Button id="start-meeting" onclick={() => goto("/capture")}>{$t("mt.start")}</Button>
  </div>
</div>

<style>
  .home {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .home-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.9rem;
    max-width: 26rem;
    padding: 1rem;
    text-align: center;
  }
  .sample {
    margin: 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
</style>
