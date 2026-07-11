<script lang="ts">
  /// The live capture: a meeting is recording on-device. You jot sparse notes (the
  /// anchor) while the transcript streams in; Stop produces the note. The recording is
  /// legible + audited - the anti-cloud-bot edge made visible.
  import { liveTranscript, liveNotes, elapsed, stopCapture, fmtTime } from "$lib/stores/meeting";
  import { t, dir } from "$lib/i18n/messages";
  import TranscriptPanel from "./TranscriptPanel.svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { Square } from "lucide-svelte";

  let notesEl = $state<HTMLTextAreaElement | null>(null);
  $effect(() => {
    notesEl?.focus();
  });
</script>

<div class="cap" dir={$dir}>
  <header class="cap-head">
    <div class="rec">
      <span class="dot" aria-hidden="true"></span>
      <span class="rec-label">{$t("mt.recording")}</span>
      <span class="elapsed">{fmtTime($elapsed)}</span>
    </div>
    <Button variant="outline" size="sm" class="ms-auto" onclick={() => stopCapture()}>
      <Square size={13} strokeWidth={2} /> {$t("mt.stop")}
    </Button>
  </header>

  <div class="cap-body">
    <div class="notes-pane">
      <p class="pane-label">{$t("mt.yourNotes")}</p>
      <Textarea
        bind:ref={notesEl}
        bind:value={$liveNotes}
        rows={6}
        maxRows={20}
        placeholder={$t("mt.notes.placeholder")}
        aria-label={$t("mt.yourNotes")}
        spellcheck="false"
      />
    </div>
    <TranscriptPanel transcript={$liveTranscript} />
  </div>
</div>

<style>
  .cap {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  .cap-head {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.9rem 1.5rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    flex-shrink: 0;
  }
  .rec {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 999px;
    background: var(--color-warning, #d0a54a);
    animation: rec-pulse 1.4s ease-in-out infinite;
  }
  @keyframes rec-pulse {
    0%,
    100% {
      opacity: 0.4;
    }
    50% {
      opacity: 1;
    }
  }
  .rec-label {
    font-size: var(--text-base);
    font-weight: 500;
  }
  .elapsed {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .cap-body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 24rem;
  }
  .notes-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 1.5rem;
    max-width: 44rem;
  }
  .pane-label {
    margin: 0 0 0.5rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
