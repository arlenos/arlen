<script lang="ts">
  /// The transcript: the first-class verification source. Adjacent same-speaker
  /// segments fold into utterances; clicking one highlights it (the coarse
  /// click-to-transcript). Speaker labels are diarization DRAFTS (~20% wrong on
  /// real meetings), so they are click-to-rename right here, and a low-confidence
  /// run says it is unsure instead of asserting.
  import { mergeAdjacent, fmtTime, speakerNum, speakerNames, relabelSpeaker } from "$lib/stores/meeting";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { t } from "$lib/i18n/messages";
  import type { Transcript } from "$lib/contract";

  let {
    transcript,
    activeStart = null,
    onseek,
    renamable = false,
  }: {
    transcript: Transcript;
    activeStart?: number | null;
    onseek?: (startMs: number) => void;
    /// Speaker labels become click-to-rename (the note view; live capture keeps
    /// them read-only while diarization has not run).
    renamable?: boolean;
  } = $props();

  const utterances = $derived(mergeAdjacent(transcript.segments));

  function speakerDisplay(label: string | undefined): string {
    if (label && $speakerNames[label]) return $speakerNames[label];
    const num = speakerNum(label);
    return num === null ? $t("mt.speaker.generic") : $t("mt.speaker", { n: num });
  }

  let renaming = $state<string | null>(null);
  let nameDraft = $state("");
  function beginRename(label: string | undefined) {
    if (!renamable || !label) return;
    renaming = label;
    nameDraft = $speakerNames[label] ?? "";
  }
  function commitRename(label: string) {
    renaming = null;
    void relabelSpeaker(label, nameDraft);
  }

  // The active segment (a source_segment's start_ms) may sit inside a merged
  // utterance rather than at its start, so match by time containment, and scroll
  // the highlighted utterance into view when a claim/item jumps here.
  let bodyEl = $state<HTMLElement | undefined>();
  function isActive(u: { start_ms: number; end_ms: number }): boolean {
    return activeStart !== null && activeStart >= u.start_ms && activeStart < u.end_ms;
  }
  $effect(() => {
    if (activeStart === null) return;
    bodyEl?.querySelector<HTMLElement>(".utt.active")?.scrollIntoView({ block: "nearest" });
  });
</script>

<aside class="tp">
  <div class="tp-head">{$t("mt.transcript")}</div>
  <div class="tp-body" bind:this={bodyEl}>
    {#each utterances as u (u.start_ms)}
      <div class="utt" class:active={isActive(u)}>
        <span class="utt-meta">
          <span class="utt-time">{fmtTime(u.start_ms)}</span>
          {#if renaming !== null && renaming === u.speaker}
            <Input
              value={nameDraft}
              aria-label={$t("mt.speaker.rename")}
              class="h-6 w-28 text-xs"
              oninput={(e) => (nameDraft = e.currentTarget.value)}
              onkeydown={(e) => {
                if (e.key === "Enter" && u.speaker) commitRename(u.speaker);
                if (e.key === "Escape") renaming = null;
              }}
            />
          {:else if renamable && u.speaker}
            <button
              type="button"
              class="utt-speaker renamable"
              title={$t("mt.speaker.rename")}
              onclick={() => beginRename(u.speaker)}
            >
              {speakerDisplay(u.speaker)}
            </button>
          {:else}
            <span class="utt-speaker">{speakerDisplay(u.speaker)}</span>
          {/if}
          {#if u.confidence !== undefined && u.confidence < 0.8}
            <span class="utt-unsure">{$t("mt.speaker.unsure")}</span>
          {/if}
        </span>
        <button type="button" class="utt-text" onclick={() => onseek?.(u.start_ms)}>
          {u.text}
        </button>
      </div>
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
    border-radius: var(--radius-input);
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
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .utt-speaker {
    border: none;
    padding: 0;
    background: transparent;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .utt-speaker.renamable {
    cursor: pointer;
    text-decoration: underline dotted;
    text-underline-offset: 3px;
  }
  .utt-speaker.renamable:hover {
    color: var(--color-fg-primary);
  }
  .utt-unsure {
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .utt-text {
    border: none;
    padding: 0;
    background: transparent;
    text-align: start;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
    cursor: pointer;
  }
  .utt-text:hover {
    color: var(--color-fg-primary);
  }
</style>
