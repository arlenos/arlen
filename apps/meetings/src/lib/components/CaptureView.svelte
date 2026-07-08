<script lang="ts">
  /// The live capture: a meeting is recording on-device. You jot sparse notes (the
  /// anchor) while the transcript streams in; Stop produces the note. The recording is
  /// legible + audited - the anti-cloud-bot edge made visible.
  import { liveTranscript, liveNotes, elapsed, stopCapture, fmtTime } from "$lib/stores/meeting";
  import TranscriptPanel from "./TranscriptPanel.svelte";
  import { ShieldCheck, Square } from "lucide-svelte";

  let notesEl = $state<HTMLTextAreaElement | null>(null);
  $effect(() => {
    notesEl?.focus();
  });
</script>

<div class="cap">
  <header class="cap-head">
    <div class="rec">
      <span class="dot" aria-hidden="true"></span>
      <span class="rec-label">Recording</span>
      <span class="elapsed">{fmtTime($elapsed)}</span>
    </div>
    <span class="sovereign"><ShieldCheck size={13} strokeWidth={2} /> On this device, in your audit log</span>
    <button type="button" class="stop" onclick={() => stopCapture()}>
      <Square size={13} strokeWidth={2} /> Stop
    </button>
  </header>

  <div class="cap-body">
    <div class="notes-pane">
      <p class="pane-label">Your notes</p>
      <textarea
        bind:this={notesEl}
        bind:value={$liveNotes}
        class="notes"
        placeholder="Jot what matters. The AI fills the rest in from the recording."
        aria-label="Your notes"
        spellcheck="false"
      ></textarea>
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
    font-size: 0.875rem;
    font-weight: 500;
  }
  .elapsed {
    font-size: 0.8125rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .sovereign {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin-inline-start: auto;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .stop {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.8rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: 0.8125rem;
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .stop:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
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
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .notes {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: none;
    background: transparent;
    resize: none;
    font: inherit;
    font-size: 0.9375rem;
    line-height: 1.6;
    color: var(--color-fg-primary);
    outline: none;
  }
  .notes::placeholder {
    color: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
  }
</style>
