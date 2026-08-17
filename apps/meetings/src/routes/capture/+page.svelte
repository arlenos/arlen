<script lang="ts">
  /// Capturing: the honest lifecycle. The head says plainly what is happening
  /// (recording locally, nothing joins the call), transcription is its own
  /// opt-outable step (recording and transcribing are different consents), and
  /// your notes are the anchor the AI enhances after. Stop produces the note and
  /// lands on its route.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import MeetingShell from "$lib/components/MeetingShell.svelte";
  import TranscriptPanel from "$lib/components/TranscriptPanel.svelte";
  import { t, dir } from "$lib/i18n/messages";
  import {
    liveTranscript,
    liveNotes,
    transcribe,
    elapsed,
    fmtTime,
    startCapture,
    stopCapture,
    captureUnavailable,
    stopFailed,
  } from "$lib/stores/meeting";

  // DEV only, and refused in a build: the refused-stop line cannot be reached by
  // hand. Reaching it needs a capture that started (so a backend) and then a stop
  // that failed (so no backend), which no single session gives you. Pinning it is
  // how it gets looked at - the same affordance the viewers app already carries,
  // and for the same reason. A shipped app must not be talkable into claiming a
  // microphone is on.
  const pinned = import.meta.env.DEV ? page.url.searchParams.get("state") : null;

  onMount(() => {
    if (pinned === "stop-failed") {
      stopFailed.set(true);
      return;
    }
    void startCapture();
  });

  let notesEl = $state<HTMLTextAreaElement | null>(null);
  $effect(() => {
    notesEl?.focus();
  });

  async function stop() {
    // Only navigate to a note that exists. A failed summarise used to hand back
    // the fixture note, so this went to it regardless; now it returns false and
    // the capture surface stays put with everything the user typed still in it.
    if (await stopCapture()) await goto("/meeting/live");
  }
</script>

<div class="page" dir={$dir}>
  <MeetingShell>
    {#snippet head()}
      <div class="cap-head">
        <!-- The red dot and the clock are a claim that a microphone is on. They
             render only when one is, which is why this branch exists at all: a
             refused capture used to show both, ticking, over silence. -->
        {#if $stopFailed}
          <!-- The retry matters more here than in the branch below it. A refused
               START leaves nothing running and the user can walk away; a refused
               STOP may leave the microphone live, so the one thing this surface
               owes them is another way to turn it off. Without it the honest
               sentence was a dead end. -->
          <span class="rec-failed" role="alert">{$t("mt.stopFailed")}</span>
          <Button variant="outline" size="sm" onclick={() => void stop()}>
            {$t("mt.captureRetry")}
          </Button>
        {:else if $captureUnavailable}
          <span class="rec-failed" role="alert">{$t("mt.captureUnavailable")}</span>
          <Button variant="outline" size="sm" onclick={() => void startCapture()}>
            {$t("mt.captureRetry")}
          </Button>
        {:else}
          <span class="rec">
            <span class="dot" aria-hidden="true"></span>
            {$t("mt.recording")}
            <span class="time">{fmtTime($elapsed)}</span>
          </span>
          <span class="consent">{$t("mt.consent")}</span>
          <!-- Inside the branch, for the reason stated above it: Stop is an offer
               to end something, and after a refused capture there is nothing to
               end. It sat outside this `if` while the dot and the clock were
               moved in, so a screen saying "Nothing is being captured" still
               carried an enabled Stop. -->
          <Button variant="outline" size="sm" id="stop" onclick={stop}>{$t("mt.stop")}</Button>
        {/if}
        <label class="transcribe">
          <Switch value={$transcribe} size="sm" ariaLabel={$t("mt.transcribe")} onchange={(v) => transcribe.set(v)} />
          {$t("mt.transcribe")}
        </label>

      </div>
    {/snippet}
    {#snippet content()}
      <div class="notes">
        <span class="sec-label">{$t("mt.yourNotes")}</span>
        <Textarea
          bind:ref={notesEl}
          bind:value={$liveNotes}
          rows={8}
          placeholder={$t("mt.notes.placeholder")}
          aria-label={$t("mt.yourNotes")}
        />
      </div>
    {/snippet}
    {#snippet rail()}
      {#if $transcribe}
        <TranscriptPanel transcript={$liveTranscript} />
      {:else}
        <aside class="tp-off">
          <p>{$t("mt.transcribe.off")}</p>
        </aside>
      {/if}
    {/snippet}
  </MeetingShell>
</div>

<style>
  .page {
    height: 100%;
    min-height: 0;
  }
  .cap-head {
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .rec {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .rec-failed {
    display: inline-flex;
    align-items: center;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-error, #f87171);
  }
  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: var(--radius-full, 9999px);
    background: var(--color-warning);
    animation: rec-pulse 1.4s ease-in-out infinite;
  }
  @keyframes rec-pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot {
      animation: none;
    }
  }
  .time {
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  /* The honest line: what is happening, said where it happens. */
  .consent {
    flex: 1;
    min-width: 12rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .transcribe {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 75%, transparent);
    cursor: pointer;
  }
  .notes {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .sec-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .tp-off {
    display: flex;
    align-items: flex-start;
    padding: 1rem;
    border-inline-start: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .tp-off p {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
