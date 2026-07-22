<script lang="ts">
  /// The meeting note: the Granola verifiable document. Your own notes (black, the
  /// anchor) sit against the AI summary (greyed, "from the recording"), both checkable
  /// against the transcript on the right. On-device capture is the edge over cloud
  /// bots. Fixture-backed; the ASR capture, the summarize engine, the KG-file store and
  /// the editor handoff are coder seams.
  import { onMount } from "svelte";
  import { meeting, phase, openInEditor, loadMeetings } from "$lib/stores/meeting";
  import { t, dir } from "$lib/i18n/messages";
  import TranscriptPanel from "$lib/components/TranscriptPanel.svelte";
  import CaptureView from "$lib/components/CaptureView.svelte";
  import MeetingList from "$lib/components/MeetingList.svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { SquareArrowOutUpRight, CalendarPlus, Square } from "lucide-svelte";

  let notes = $state("");
  let activeStart = $state<number | null>(null);

  onMount(loadMeetings);

  // Keep the editable anchor in sync when the produced note arrives.
  $effect(() => {
    if ($meeting) notes = $meeting.humanNotes;
  });
</script>

{#if $phase === "idle"}
  <MeetingList />
{:else if $phase === "capturing"}
  <CaptureView />
{:else if $meeting}
  {@const n = $meeting.note}
  <div class="app" dir={$dir}>
    {#if $meeting.mocked}
      <!-- This fixture is a whole meeting RECORD: a title, named participants
           and quotes attributed to them. Unlabelled it reads as minutes of a
           conversation that never happened. -->
      <p class="sample">{$t("mt.sample")}</p>
    {/if}
    <header class="head">
      <div class="head-main">
        <h1 class="title">{n.title}</h1>
        <div class="parts">
          {#each n.participants as p (p)}<Badge variant="secondary">{p}</Badge>{/each}
        </div>
      </div>
      <div class="head-side">
        <Button variant="outline" size="sm" onclick={openInEditor}>
          <SquareArrowOutUpRight size={13} strokeWidth={2} /> {$t("mt.open")}
        </Button>
      </div>
    </header>

    <div class="body">
      <div class="note">
        <section class="sec">
          <p class="sec-label">{$t("mt.yourNotes")}</p>
          <Textarea bind:value={notes} rows={3} maxRows={12} aria-label={$t("mt.yourNotes")} spellcheck="false" />
        </section>

        <section class="sec">
          <p class="sec-label">{$t("mt.summary")} <span class="from">{$t("mt.summary.from")}</span></p>
          {#if n.summary_claims && n.summary_claims.length}
            <p class="summary">
              {#each n.summary_claims as claim, i (i)}
                {#if claim.source_segment !== undefined && n.transcript.segments[claim.source_segment]}
                  {@const start = n.transcript.segments[claim.source_segment].start_ms}
                  <button type="button" class="claim linked" onclick={() => (activeStart = start)}>{claim.text}</button>
                {:else}
                  <span class="claim">{claim.text}</span>
                {/if}{" "}
              {/each}
            </p>
          {:else}
            <p class="summary">{n.summary}</p>
          {/if}
          <p class="grounded">{$t("mt.grounded")}</p>
        </section>

        <section class="sec">
          <p class="sec-label">{$t("mt.actionItems")}</p>
          {#if n.action_items.length === 0}
            <p class="empty">{$t("mt.actionItems.none")}</p>
          {:else}
            <ul class="items">
              {#each n.action_items as item, i (i)}
                <li class="item">
                  <span class="box" aria-hidden="true"><Square size={14} strokeWidth={2} /></span>
                  {#if item.source_segment !== undefined && n.transcript.segments[item.source_segment]}
                    {@const start = n.transcript.segments[item.source_segment].start_ms}
                    <button type="button" class="item-text linked" onclick={() => (activeStart = start)}>{item.text}</button>
                  {:else}
                    <span class="item-text">{item.text}</span>
                  {/if}
                  {#if item.owner}<Badge variant="secondary">@{item.owner}</Badge>{/if}
                  <Button variant="ghost" size="sm" class="ms-auto" title={$t("mt.add.title")}>
                    <CalendarPlus size={13} strokeWidth={2} /> {$t("mt.add")}
                  </Button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>

      <TranscriptPanel transcript={n.transcript} {activeStart} onseek={(s) => (activeStart = s)} />
    </div>
  </div>
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  /* Above the title, because it qualifies the title, the participants and every
     quote below them. */
  .sample {
    margin: 0;
    padding: 0.8rem 1.5rem 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
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
    font-size: var(--text-lg);
    font-weight: 600;
  }
  .parts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .head-side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.5rem;
    flex-shrink: 0;
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
    font-size: var(--text-2xs);
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
  /* The AI summary: greyed, clearly not yours - the verifiable-merge signal. */
  .summary {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.65;
    color: color-mix(in srgb, var(--color-fg-primary) 58%, transparent);
  }
  .grounded {
    margin: 0.5rem 0 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
  }
  .empty {
    margin: 0;
    font-size: var(--text-base);
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
    font-size: var(--text-md);
  }
  .box {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .item-text {
    color: var(--color-fg-primary);
  }

  /* A summary claim / action item grounded to a transcript segment: an inline,
     button-reset span that jumps to + highlights its source. Ungrounded claims
     stay plain (no affordance, no fabricated citation). */
  .claim {
    font: inherit;
    color: inherit;
  }
  .claim.linked,
  .item-text.linked {
    border: none;
    background: none;
    padding: 0;
    margin: 0;
    font: inherit;
    line-height: inherit;
    text-align: start;
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--color-fg-primary) 22%, transparent);
    text-underline-offset: 3px;
    transition: text-decoration-color var(--duration-fast, 150ms) ease, color var(--duration-fast, 150ms) ease;
  }
  .claim.linked {
    color: inherit;
  }
  .item-text.linked {
    color: var(--color-fg-primary);
  }
  .claim.linked:hover,
  .item-text.linked:hover {
    text-decoration-color: currentColor;
  }
  .claim.linked:hover {
    color: color-mix(in srgb, var(--color-fg-primary) 85%, transparent);
  }
</style>
