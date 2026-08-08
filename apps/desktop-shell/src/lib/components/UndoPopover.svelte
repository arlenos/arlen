<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The unified recent-actions panel (CAH-4): every producer's reversible
  /// acts in one list, newest first - action + its inverse, and a marked
  /// point of no return where there is none. "Undo last" takes back the
  /// newest reversible act in one gesture (Raskin: never a warning where an
  /// undo will do). Honest vocabulary: entries name the inverse as the act it
  /// performs ("Put back", "Restore"), never a blanket "Undo everything".
  import { Undo2, Check } from "lucide-svelte";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import { undoHistory, undoMocked, undoUnavailable, enact } from "$lib/stores/undoHistory";

  // Compact ages so the row stays one calm line ("now", "4m", "2h").
  function ago(at: number): string {
    const s = Math.max(0, Math.floor(Date.now() / 1000) - at);
    if (s < 90) return "now";
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m`;
    return `${Math.floor(m / 60)}h`;
  }
</script>

<ShellPopover id="undo" width={380} right={116} bodyPadding="12px" bodyGap="8px">
  {#snippet header()}
    <PopoverHeader icon={Undo2} title="{$t("sh.undo.title")}" />
  {/snippet}

  {#if $undoMocked}
    <p class="undo-sample">{$t("sh.undo.mocked")}</p>
  {/if}

  {#if $undoUnavailable}
    <!-- The read failed in a real session. Say so, and show no rows: a populated
         panel of actions nobody performed invites a click on one of them. -->
    <p class="undo-empty">{$t("sh.undo.unavailable")}</p>
  {:else if $undoHistory && $undoHistory.length === 0}
    <p class="undo-empty">{$t("sh.undo.empty")}</p>
  {:else if $undoHistory}
    <div class="undo-list">
      {#each $undoHistory as e (e.opId)}
        <div class="undo-row" class:done={e.state === "done"}>
          <span class="undo-text">
            <span class="undo-verb">{e.verb}</span>
            <span class="undo-object">{e.object}</span>
          </span>
          {#if e.reversibility === "irreversible"}
            <span class="undo-ponr">{$t("sh.undo.irreversible")}</span>
          {:else if e.state === "done"}
            <span class="undo-done">
              <Check size={13} strokeWidth={2} />
              {$t("sh.undo.done")}
            </span>
          {:else}
            <button class="undo-act" disabled={e.state === "enacting"} onclick={() => void enact(e.opId)}>
              {e.inverseLabel ?? "Undo"}
            </button>
          {/if}
          <span class="undo-time">{ago(e.at)}</span>
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

  .undo-empty {
    margin: 0.25rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .undo-list {
    display: flex;
    flex-direction: column;
  }
  /* One act per line: the sentence, the inverse (or the point-of-no-return
     marker), then when. The age sits LAST as a fixed trailing column so it
     reads as a clean column; the action's ragged edge hides against it
     because both are right-aligned. */
  .undo-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 2.2rem;
    align-items: baseline;
    column-gap: 0.5rem;
    padding: 0.3rem 0.25rem;
    border-radius: var(--radius-chip, 4px);
  }
  .undo-row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .undo-row.done {
    opacity: 0.55;
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
