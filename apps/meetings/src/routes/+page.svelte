<script lang="ts">
  /// Home: the sidebar carries the meeting list now, so this surface is the
  /// quiet start point - one action, one hint. Rows and history live in the
  /// rail, always visible, never one Back away.
  import { goto } from "$app/navigation";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t, dir } from "$lib/i18n/messages";
  import {
    meetings,
    meetingsMocked,
    meetingsUnavailable,
  } from "$lib/stores/meeting";
</script>

<div class="home" dir={$dir}>
  <div class="home-center">
    <!-- NO sample caveat here, and no refusal either: both are about the list,
         and the rail says each once, beside the rows (design-system.md 6.11,
         thread two: the refusal sits at the top of the surface that failed).
         This pane used to repeat the refusal in grey, so the screen said it
         twice. What stays is this pane's own offer. When the read failed,
         "No meetings" would answer a question the rail just said it could not
         answer, so the hint goes and the button stands alone. -->
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
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
</style>
