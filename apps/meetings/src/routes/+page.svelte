<script lang="ts">
  /// The meeting note: the Granola verifiable document. Your own notes (black, the
  /// anchor) sit against the AI summary (greyed, "from the recording"), both checkable
  /// against the transcript on the right. On-device capture is the edge over cloud
  /// bots. Fixture-backed; the ASR capture, the summarize engine, the KG-file store and
  /// the editor handoff are coder seams.
  import { onMount } from "svelte";
  import { meeting, loadMeeting, openInEditor } from "$lib/stores/meeting";
  import TranscriptPanel from "$lib/components/TranscriptPanel.svelte";
  import { ShieldCheck, SquareArrowOutUpRight, CalendarPlus, Square } from "lucide-svelte";

  let notes = $state("");
  let activeStart = $state<number | null>(null);

  onMount(loadMeeting);
  // Keep the editable anchor in sync when the meeting loads.
  $effect(() => {
    if ($meeting) notes = $meeting.humanNotes;
  });
</script>

<div class="app">
  {#if $meeting}
    {@const n = $meeting.note}
    <header class="head">
      <div class="head-main">
        <h1 class="title">{n.title}</h1>
        <div class="parts">
          {#each n.participants as p (p)}<span class="chip">{p}</span>{/each}
        </div>
      </div>
      <div class="head-side">
        <span class="sovereign"><ShieldCheck size={13} strokeWidth={2} /> Captured on this device, in your audit log</span>
        <button type="button" class="open" onclick={openInEditor}>
          <SquareArrowOutUpRight size={13} strokeWidth={2} /> Open in editor
        </button>
      </div>
    </header>

    <div class="body">
      <div class="note">
        <section class="sec">
          <p class="sec-label">Your notes</p>
          <textarea class="yours" bind:value={notes} aria-label="Your notes" spellcheck="false"></textarea>
        </section>

        <section class="sec">
          <p class="sec-label">Summary <span class="from">from the recording</span></p>
          <p class="summary">{n.summary}</p>
          <p class="grounded">Every line is drawn from the transcript. Read it on the right to check.</p>
        </section>

        <section class="sec">
          <p class="sec-label">Action items</p>
          {#if n.action_items.length === 0}
            <p class="empty">None captured.</p>
          {:else}
            <ul class="items">
              {#each n.action_items as item, i (i)}
                <li class="item">
                  <span class="box" aria-hidden="true"><Square size={14} strokeWidth={2} /></span>
                  <span class="item-text">{item.text}</span>
                  {#if item.owner}<span class="owner">@{item.owner}</span>{/if}
                  <button type="button" class="act" title="Add to your calendar (asks first)">
                    <CalendarPlus size={13} strokeWidth={2} /> Add
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>

      <TranscriptPanel transcript={n.transcript} {activeStart} onseek={(s) => (activeStart = s)} />
    </div>
  {/if}
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.1rem 1.5rem 1rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    flex-shrink: 0;
  }
  .title {
    margin: 0 0 0.4rem;
    font-size: 1.0625rem;
    font-weight: 600;
  }
  .parts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .chip {
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .head-side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.5rem;
    flex-shrink: 0;
  }
  .sovereign {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .open {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.7rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-primary) 75%, transparent);
    cursor: pointer;
  }
  .open:hover {
    color: var(--color-fg-primary);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 24rem;
  }
  .note {
    min-height: 0;
    overflow-y: auto;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1.75rem;
    max-width: 44rem;
  }
  .sec-label {
    margin: 0 0 0.5rem;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .from {
    margin-inline-start: 0.4rem;
    font-weight: 400;
    letter-spacing: 0;
    text-transform: none;
    color: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
  }
  /* Your notes: full-strength, they are the anchor + editable. */
  .yours {
    width: 100%;
    min-height: 5rem;
    border: none;
    background: transparent;
    resize: vertical;
    font: inherit;
    font-size: 0.9375rem;
    line-height: 1.6;
    color: var(--color-fg-primary);
    outline: none;
  }
  /* The AI summary: greyed, clearly not yours - the verifiable-merge signal. */
  .summary {
    margin: 0;
    font-size: 0.9375rem;
    line-height: 1.65;
    color: color-mix(in srgb, var(--color-fg-primary) 58%, transparent);
  }
  .grounded {
    margin: 0.5rem 0 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
  }
  .empty {
    margin: 0;
    font-size: 0.875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.9375rem;
  }
  .box {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .item-text {
    color: var(--color-fg-primary);
  }
  .owner {
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .act {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.2rem 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    cursor: pointer;
  }
  .act:hover {
    color: var(--color-fg-primary);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
</style>
