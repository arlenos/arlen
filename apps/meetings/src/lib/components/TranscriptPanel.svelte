<script lang="ts">
  /// The transcript: the first-class verification source. Adjacent same-speaker
  /// segments are folded into utterances (per the contract). Clicking one highlights
  /// it - the coarse click-to-transcript. Per-claim span provenance (jump from a
  /// specific summary sentence to its exact span) is a coder seam.
  import { mergeAdjacent, fmtTime, speakerNum } from "$lib/stores/meeting";
  import { t } from "$lib/i18n/messages";
  import type { Transcript } from "$lib/contract";

  let {
    transcript,
    activeStart = null,
    onseek,
  }: {
    transcript: Transcript;
    activeStart?: number | null;
    onseek?: (startMs: number) => void;
  } = $props();

  const utterances = $derived(mergeAdjacent(transcript.segments));
</script>

<aside class="tp">
  <div class="tp-head">{$t("mt.transcript")}</div>
  <div class="tp-body">
    {#each utterances as u (u.start_ms)}
      {@const num = speakerNum(u.speaker)}
      <button
        type="button"
        class="utt"
        class:active={activeStart === u.start_ms}
        onclick={() => onseek?.(u.start_ms)}
      >
        <span class="utt-meta">
          <span class="utt-time">{fmtTime(u.start_ms)}</span>
          <span class="utt-speaker">{num === null ? $t("mt.speaker.generic") : $t("mt.speaker", { n: num })}</span>
        </span>
        <span class="utt-text">{u.text}</span>
      </button>
    {/each}
  </div>
</aside>

<style>
  .tp {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-inline-start: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .tp-head {
    flex-shrink: 0;
    padding: 0.85rem 1rem 0.6rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .tp-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 0.5rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .utt {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.5rem 0.6rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .utt:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .utt.active {
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
  }
  .utt-meta {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: var(--text-2xs);
  }
  .utt-time {
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .utt-speaker {
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .utt-text {
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }
</style>
