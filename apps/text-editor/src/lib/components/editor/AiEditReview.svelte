<script lang="ts">
  /// The sovereign gated AI-edit review: the assistant's proposed change, shown per
  /// hunk, never silent. The gate is made LEGIBLE - you see which hunks the AI
  /// applied on its own (reversible, undoable) versus which it is holding for your
  /// confirm (irreversible/external). The opposite of a silent apply.
  import {
    proposal,
    acceptHunk,
    rejectHunk,
    undoHunk,
    dismiss,
    type EditHunk,
  } from "$lib/stores/aiEdit";
  import { ScopeChip } from "@arlen/ui-kit/components/ui/scope-chip";
  import { Check, Undo2, AlertTriangle, X } from "lucide-svelte";

  function badge(h: EditHunk): { text: string; tone: "applied" | "held" | "muted" } {
    if (h.status === "undone") return { text: "Undone", tone: "muted" };
    if (h.status === "rejected") return { text: "Rejected", tone: "muted" };
    if (h.status === "pending") return { text: "Needs your confirmation", tone: "held" };
    return { text: h.gate === "auto" ? "Applied on its own" : "Applied", tone: "applied" };
  }
</script>

{#if $proposal}
  {@const p = $proposal}
  <aside class="review">
    <header class="head">
      <div class="who">
        <span class="who-name">{p.principal}</span>
        <ScopeChip label={p.scope} />
      </div>
      <button type="button" class="close" aria-label="Dismiss" onclick={dismiss}>
        <X size={15} strokeWidth={2} />
      </button>
    </header>

    <p class="prompt">You asked: {p.prompt}</p>

    <div class="hunks">
      {#each p.hunks as h, i (i)}
        {@const b = badge(h)}
        <div class="hunk" data-status={h.status}>
          <div class="hunk-head">
            <span class="hunk-title">{h.header}</span>
            <span class="hunk-badge" data-tone={b.tone}>
              {#if b.tone === "held"}<AlertTriangle size={12} strokeWidth={2} />{/if}
              {#if b.tone === "applied"}<Check size={12} strokeWidth={2.5} />{/if}
              {b.text}
            </span>
          </div>

          <div class="diff">
            {#each h.lines as line (line.text)}
              <div class="row {line.kind}">
                <span class="rg">{line.kind === "add" ? "+" : line.kind === "del" ? "-" : ""}</span>
                <span class="rc">{line.text || " "}</span>
              </div>
            {/each}
          </div>

          <div class="hunk-foot">
            <span class="rationale">{h.rationale}</span>
            <span class="actions">
              {#if h.status === "pending"}
                <button type="button" class="act reject" onclick={() => rejectHunk(i)}>Reject</button>
                <button type="button" class="act accept" onclick={() => acceptHunk(i)}>Accept</button>
              {:else if h.status === "applied"}
                <button type="button" class="act" onclick={() => undoHunk(i)}>
                  <Undo2 size={13} strokeWidth={2} /> Undo
                </button>
              {/if}
            </span>
          </div>
        </div>
      {/each}
    </div>

    <p class="foot">Every change is logged, and you can undo any of it. Turn the assistant off in Settings.</p>
  </aside>
{/if}

<style>
  .review {
    width: 24rem;
    flex-shrink: 0;
    padding: 1.25rem 1.15rem;
    border-left: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-y: auto;
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.6rem;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  .who-name {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .close {
    flex-shrink: 0;
    display: inline-flex;
    padding: 0.2rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    cursor: pointer;
  }
  .close:hover {
    color: var(--color-fg-primary);
  }
  .prompt {
    margin: 0 0 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .hunks {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }
  .hunk {
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card, 12px);
    overflow: hidden;
  }
  .hunk[data-status="undone"],
  .hunk[data-status="rejected"] {
    opacity: 0.55;
  }
  .hunk-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.5rem 0.65rem;
  }
  .hunk-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .hunk-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.6875rem;
    white-space: nowrap;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .hunk-badge[data-tone="applied"] {
    color: var(--color-success, #8fae74);
  }
  .hunk-badge[data-tone="held"] {
    color: var(--color-warning, #d0a54a);
  }

  .diff {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
    line-height: 1.5;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-x: auto;
  }
  .row {
    display: flex;
    white-space: pre-wrap;
  }
  .rg {
    flex-shrink: 0;
    width: 1.1rem;
    padding-left: 0.35rem;
    user-select: none;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .rc {
    padding-right: 0.5rem;
    word-break: break-word;
  }
  .row.add {
    background: color-mix(in srgb, var(--color-success, #8fae74) 14%, transparent);
  }
  .row.add .rg {
    color: var(--color-success, #8fae74);
  }
  .row.del {
    background: color-mix(in srgb, var(--color-error, #c96a6a) 14%, transparent);
  }
  .row.del .rg {
    color: var(--color-error, #c96a6a);
  }

  .hunk-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    padding: 0.5rem 0.65rem;
  }
  .rationale {
    font-size: 0.6875rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--color-fg-primary) 48%, transparent);
  }
  .actions {
    display: inline-flex;
    gap: 0.35rem;
    flex-shrink: 0;
  }
  .act {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.55rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 15%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: 0.75rem;
    color: var(--color-fg-primary);
    cursor: pointer;
    white-space: nowrap;
  }
  .act:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .act.accept {
    border-color: transparent;
    background: var(--color-fg-primary);
    color: var(--color-bg-app, #0f0f0f);
  }

  .foot {
    margin: 1.25rem 0 0;
    font-size: 0.6875rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
</style>
