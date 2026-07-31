<script lang="ts">
  /// The unified recent-actions panel (CAH-4): every producer's reversible
  /// acts in one list, newest first - action + its inverse, and a marked
  /// point of no return where there is none. "Undo last" takes back the
  /// newest reversible act in one gesture (Raskin: never a warning where an
  /// undo will do). Honest vocabulary: entries name the inverse as the act it
  /// performs ("Put back", "Restore"), never a blanket "Undo everything".
  import { Undo2, Check, AlertTriangle, Folder, Sparkles, SquareTerminal, SlidersHorizontal } from "lucide-svelte";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import {
    undoHistory,
    undoMocked,
    enact,
    enactLast,
    type UndoEntry,
  } from "$lib/stores/undoHistory";

  // Producer as a small icon, not a text tag - the sentence carries the story.
  const PRODUCER_ICONS: Record<UndoEntry["producer"], typeof Folder> = {
    files: Folder,
    agent: Sparkles,
    terminal: SquareTerminal,
    settings: SlidersHorizontal,
  };
  const PRODUCER_NAMES: Record<UndoEntry["producer"], string> = {
    files: "Files",
    agent: "The assistant",
    terminal: "Terminal",
    settings: "Settings",
  };

  // Compact ages so the row stays one calm line ("now", "4m", "2h").
  function ago(at: number): string {
    const s = Math.max(0, Math.floor(Date.now() / 1000) - at);
    if (s < 90) return "now";
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m`;
    return `${Math.floor(m / 60)}h`;
  }

  const canUndoLast = $derived(
    ($undoHistory ?? []).some((e) => e.state === "ready" && e.reversibility !== "irreversible")
  );
</script>

<ShellPopover id="undo" width={360} right={116} bodyPadding="12px" bodyGap="8px">
  {#snippet header()}
    <PopoverHeader icon={Undo2} title="Recent actions" />
  {/snippet}

  {#if $undoMocked}
    <p class="undo-sample">Example actions - nothing here really ran.</p>
  {/if}

  <button class="undo-last" disabled={!canUndoLast} onclick={() => void enactLast()}>
    <Undo2 size={14} strokeWidth={2} />
    Undo last
  </button>

  {#if $undoHistory && $undoHistory.length === 0}
    <p class="undo-empty">Nothing to take back right now.</p>
  {:else if $undoHistory}
    <div class="undo-list">
      {#each $undoHistory as e (e.opId)}
        {@const ProdIcon = PRODUCER_ICONS[e.producer]}
        <div class="undo-row" class:done={e.state === "done"}>
          <span class="undo-prod" aria-label={PRODUCER_NAMES[e.producer]}>
            <ProdIcon size={13} strokeWidth={1.75} />
          </span>
          <span class="undo-text">
            <span class="undo-verb">{e.verb}</span>
            <span class="undo-object">{e.object}</span>
          </span>
          <span class="undo-time">{ago(e.at)}</span>
          {#if e.reversibility === "irreversible"}
            <span class="undo-ponr">
              <AlertTriangle size={12} strokeWidth={2} />
              Cannot be undone
            </span>
          {:else if e.state === "done"}
            <span class="undo-done">
              <Check size={13} strokeWidth={2} />
              Done
            </span>
          {:else}
            <button class="undo-act" disabled={e.state === "enacting"} onclick={() => void enact(e.opId)}>
              {e.inverseLabel ?? "Undo"}
            </button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</ShellPopover>

<style>
  .undo-sample {
    margin: 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }

  /* The one-gesture undo: the panel's primary act, full width, calm. */
  .undo-last {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    width: 100%;
    height: var(--height-control-prominent, 34px);
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .undo-last:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
  }
  .undo-last:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .undo-empty {
    margin: 0.25rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .undo-list {
    display: flex;
    flex-direction: column;
  }
  /* One act per row: producer tag, the sentence, when, and the inverse (or
     the point-of-no-return marker). The time column is content-sized but the
     row grid keeps one seam because the action column is fixed. */
  .undo-row {
    display: grid;
    grid-template-columns: 1.25rem minmax(0, 1fr) 2.2rem 7.5rem;
    align-items: baseline;
    column-gap: 0.5rem;
    padding: 0.35rem 0.25rem;
    border-radius: var(--radius-chip, 4px);
  }
  .undo-row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .undo-row.done {
    opacity: 0.55;
  }
  .undo-prod {
    display: inline-flex;
    align-self: start;
    margin-top: 0.2rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  /* The sentence wraps rather than truncating - the object is the point. */
  .undo-text {
    min-width: 0;
    line-height: 1.35;
  }
  .undo-verb {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .undo-object {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .undo-time {
    justify-self: end;
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
    white-space: nowrap;
  }
  .undo-act {
    justify-self: end;
    border: none;
    background: transparent;
    padding: 0.125rem 0.25rem;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    cursor: pointer;
    white-space: nowrap;
    transition: color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .undo-act:hover:not(:disabled) {
    color: var(--color-fg-primary);
  }
  /* The point of no return: stated, warning-toned, never an action. */
  .undo-ponr {
    justify-self: end;
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-warning, #ca8a04) 90%, var(--color-fg-primary));
    white-space: nowrap;
  }
  .undo-done {
    justify-self: end;
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    white-space: nowrap;
  }
</style>
