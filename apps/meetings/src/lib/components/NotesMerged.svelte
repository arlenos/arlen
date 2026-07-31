<script lang="ts">
  /// The notes-anchor merge (meetings-app.md killer #2): ONE document. The user's
  /// own lines in full strength; the AI's enhancements INLINE under the line they
  /// anchor to, in the AI tint - never a separate greyed summary. Grounded
  /// enhancements click through to their transcript span; ungrounded ones get no
  /// affordance (no fabricated citation). Unanchored enhancements gather under a
  /// quiet trailing line instead of being guessed into place.
  import { CornerDownRight } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { t } from "$lib/i18n/messages";
  import type { SummaryClaim, Transcript } from "$lib/contract";

  let {
    notes,
    claims,
    transcript,
    onjump,
    onsave,
  }: {
    notes: string;
    claims: SummaryClaim[];
    transcript: Transcript;
    onjump: (startMs: number) => void;
    onsave: (text: string) => void;
  } = $props();

  const lines = $derived(notes.split("\n").filter((l) => l.trim().length > 0));
  const anchored = $derived.by(() => {
    const map = new Map<number, SummaryClaim[]>();
    for (const c of claims) {
      if (c.anchor_line !== undefined && c.anchor_line < lines.length) {
        const list = map.get(c.anchor_line) ?? [];
        list.push(c);
        map.set(c.anchor_line, list);
      }
    }
    return map;
  });
  const unanchored = $derived(claims.filter((c) => c.anchor_line === undefined || c.anchor_line >= lines.length));

  function startOf(c: SummaryClaim): number | null {
    if (c.source_segment === undefined) return null;
    const seg = transcript.segments[c.source_segment];
    return seg ? seg.start_ms : null;
  }

  // Editing the user's own lines: a plain textarea over the raw notes, saved
  // whole (the seam persists it; today nothing did - that was a real loss bug).
  let editing = $state(false);
  let draft = $state("");
  function beginEdit() {
    draft = notes;
    editing = true;
  }
  function commit() {
    editing = false;
    onsave(draft);
  }
</script>

<section class="merge" aria-label={$t("mt.yourNotes")}>
  <div class="sec-head">
    <span class="sec-label">{$t("mt.notes.merged")}</span>
    {#if !editing}
      <Button variant="ghost" size="sm" class="text-muted-foreground" id="edit-notes" onclick={beginEdit}>
        {$t("mt.notes.edit")}
      </Button>
    {/if}
  </div>

  {#if editing}
    <Textarea bind:value={draft} rows={5} aria-label={$t("mt.yourNotes")} />
    <div class="edit-actions">
      <Button size="sm" id="save-notes" onclick={commit}>{$t("mt.notes.save")}</Button>
      <Button variant="ghost" size="sm" onclick={() => (editing = false)}>{$t("mt.notes.cancel")}</Button>
    </div>
  {:else}
    <div class="doc">
      {#each lines as line, i (i)}
        <p class="you">{line}</p>
        {#each anchored.get(i) ?? [] as claim (claim.text)}
          {@const start = startOf(claim)}
          {#if start !== null}
            <button type="button" class="ai linked" onclick={() => onjump(start)}>
              <CornerDownRight size={13} strokeWidth={2} aria-hidden="true" />
              <span>{claim.text}</span>
            </button>
          {:else}
            <span class="ai">
              <CornerDownRight size={13} strokeWidth={2} aria-hidden="true" />
              <span>{claim.text}</span>
            </span>
          {/if}
        {/each}
      {/each}

      {#if unanchored.length > 0}
        <p class="also">{$t("mt.notes.also")}</p>
        {#each unanchored as claim (claim.text)}
          {@const start = startOf(claim)}
          {#if start !== null}
            <button type="button" class="ai linked" onclick={() => onjump(start)}>
              <CornerDownRight size={13} strokeWidth={2} aria-hidden="true" />
              <span>{claim.text}</span>
            </button>
          {:else}
            <span class="ai">
              <CornerDownRight size={13} strokeWidth={2} aria-hidden="true" />
              <span>{claim.text}</span>
            </span>
          {/if}
        {/each}
      {/if}
    </div>
    <p class="grounded-note">{$t("mt.grounded")}</p>
  {/if}
</section>

<style>
  .merge {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .sec-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .doc {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  /* Your line, full strength. */
  .you {
    margin: 0.35rem 0 0;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: var(--color-fg-primary);
  }
  /* The AI's enhancement: indented, in the AI tint - a different voice, visibly. */
  .ai {
    display: flex;
    align-items: flex-start;
    gap: 0.4rem;
    margin-inline-start: 1rem;
    padding: 0.1rem 0.3rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: start;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--color-ai);
  }
  .ai :global(svg) {
    flex-shrink: 0;
    margin-top: 0.2rem;
    opacity: 0.7;
  }
  .ai.linked {
    cursor: pointer;
  }
  .ai.linked:hover {
    background: color-mix(in srgb, var(--color-ai) 10%, transparent);
  }
  .also {
    margin: 0.8rem 0 0;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .grounded-note {
    margin: 0.5rem 0 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .edit-actions {
    display: flex;
    gap: 0.4rem;
  }
</style>
