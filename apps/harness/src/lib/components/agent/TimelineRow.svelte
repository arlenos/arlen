<script lang="ts">
  /// One ledger-style row on the shared grid: fixed badge column |
  /// flexible body | right-aligned end cluster (undo, time, details
  /// chevron). The surface line is one human sentence; everything raw
  /// (tool name, component, duration, reference) lives behind the chevron,
  /// honest and unabridged. Two-layer transparency.
  import { ChevronDown, Undo2 } from "@lucide/svelte";
  import type { Tone } from "$lib/ledger";

  interface Annotation {
    text: string;
    tone?: Tone;
  }

  let {
    label,
    tone = "neutral",
    subject,
    subjectMeta = [],
    detail = [],
    details = [],
    time,
    undoable = false,
    onundo,
  }: {
    /// Badge text (the row's category / state).
    label: string;
    /// Badge tone, mapped to theme tokens.
    tone?: Tone;
    /// The row's one human sentence; truncates with the full text in the
    /// tooltip.
    subject: string;
    /// Small inline annotations after the sentence (a failure marker).
    subjectMeta?: Annotation[];
    /// Visible second line (a warning's body text); wraps.
    detail?: Annotation[];
    /// Raw facts behind the chevron, shown only when expanded.
    details?: { key: string; value: string }[];
    /// Right-aligned relative time; omit for time-less rows.
    time?: string;
    /// Offer the undo action (changes only).
    undoable?: boolean;
    /// Perform the undo; resolves false when it failed.
    onundo?: () => Promise<boolean>;
  } = $props();

  let open = $state(false);
  let undoState = $state<"idle" | "working" | "failed">("idle");

  async function doUndo() {
    if (!onundo || undoState === "working") return;
    undoState = "working";
    const ok = await onundo();
    undoState = ok ? "idle" : "failed";
  }
</script>

<li class="row">
  <span class="badge" data-tone={tone}>{label}</span>
  <div class="body">
    <div class="line">
      <span class="subject" title={subject}>{subject}</span>
      {#each subjectMeta as m (m.text)}
        <span class="meta" data-tone={m.tone ?? "neutral"}>{m.text}</span>
      {/each}
    </div>
    {#if detail.length > 0}
      <div class="detail">
        {#each detail as d, i (i)}
          <span class="detail-item" data-tone={d.tone ?? "neutral"}>{d.text}</span>
        {/each}
      </div>
    {/if}
    {#if open && details.length > 0}
      <div class="raw">
        <p class="raw-title">Technical record</p>
        <dl class="raw-grid">
          {#each details as d (d.key)}
            <dt>{d.key}</dt>
            <dd>{d.value}</dd>
          {/each}
        </dl>
      </div>
    {/if}
    {#if undoState === "failed"}
      <p class="undo-note">Could not undo this change.</p>
    {/if}
  </div>
  <div class="end">
    {#if undoable}
      <button
        type="button"
        class="undo"
        disabled={undoState === "working"}
        onclick={doUndo}
      >
        <Undo2 size={13} strokeWidth={2} />
        {undoState === "working" ? "Undoing" : "Undo"}
      </button>
    {/if}
    {#if time !== undefined}
      <time class="time">{time}</time>
    {/if}
    {#if details.length > 0}
      <button
        type="button"
        class="expand"
        class:open
        aria-label={open ? "Hide details" : "Show details"}
        title={open ? "Hide details" : "Show details"}
        onclick={() => (open = !open)}
      >
        <ChevronDown size={14} strokeWidth={2} />
      </button>
    {/if}
  </div>
</li>

<style>
  .row {
    display: grid;
    grid-template-columns: 6rem minmax(0, 1fr) auto;
    column-gap: var(--space-row, 0.75rem);
    align-items: start;
    padding: 0.625rem var(--space-row, 0.75rem);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .badge[data-tone="ok"] {
    color: var(--color-success);
    background: color-mix(in srgb, var(--color-success) 14%, transparent);
  }
  .badge[data-tone="warn"] {
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 14%, transparent);
  }
  .badge[data-tone="info"] {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  }
  .body {
    min-width: 0;
  }
  /* The first body line shares the badge's height as its line box, so the
     badge, the sentence, and the time sit on one optical line. */
  .line {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    line-height: var(--height-tag, 20px);
  }
  .subject {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .meta {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  /* Failure markers are first-class, same register as the badges. */
  .meta[data-tone="warn"] {
    display: inline-flex;
    align-items: center;
    align-self: center;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 14%, transparent);
    font-weight: 500;
  }
  .detail {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.5rem;
    margin-top: 0.125rem;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .detail-item[data-tone="warn"] {
    color: var(--color-error);
  }
  /* The raw facts, expanded, framed honestly as the technical record. */
  .raw {
    margin-top: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
  }
  .raw-title {
    margin: 0 0 0.375rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .raw-grid {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 0.25rem 0.75rem;
    margin: 0;
  }
  .raw-grid dt {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .raw-grid dd {
    margin: 0;
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    word-break: break-word;
  }
  .undo-note {
    margin: 0.375rem 0 0;
    font-size: 0.75rem;
    color: var(--color-error);
  }
  .end {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-height: var(--height-tag, 20px);
  }
  .time {
    font-size: 0.75rem;
    line-height: var(--height-tag, 20px);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
  .undo {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    height: var(--height-control-compact, 24px);
    padding: 0 0.5rem;
    border: none;
    background: transparent;
    border-radius: var(--radius-button);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .undo:hover:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .undo:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .expand {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    background: transparent;
    border-radius: var(--radius-button);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
    transition:
      transform var(--duration-fast) var(--ease-out),
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .expand:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .expand.open {
    transform: rotate(180deg);
  }
</style>
