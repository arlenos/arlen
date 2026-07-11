<script lang="ts">
  /// An agent action that needs confirmation (high-impact / irreversible /
  /// external), inline in the transcript (harness-redesign-plan.md "the gate").
  /// More than Approve/Deny: "Always allow" opens a scope submenu that creates a
  /// granular, revocable capability grant (action-type x scope) - not a session
  /// toggle. A broad grant raises a standing-permission warning; a narrow one
  /// (this project) applies directly. Pull-not-push: nothing runs until chosen.
  ///
  /// When the action is a file change, the proposed **unified diff is the card's
  /// body** - review-before-apply and predict-before-act are one artifact, so the
  /// Approve button IS the act (no separate "are you sure?"). After approval the
  /// same card stays as a `done` receipt: the diff collapses and the actions
  /// become a per-change Undo (the executor compensate).
  import { t } from "$lib/i18n/messages";
  import { TriangleAlert, ChevronDown, CircleCheck, Undo2, Sparkles } from "@lucide/svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import { DiffView, parseUnifiedDiff, diffTotals, type DiffFile } from "@arlen/ui-kit/components/diff";

  let {
    title,
    detail,
    diff,
    done = false,
    auto = false,
    via,
    onapprove,
    ondeny,
    onalways,
    onundo,
  }: {
    /// The proposed action, one line ("write FILE_PART_OF (3 files → Arlen)").
    title: string;
    /// What + why (predict-before-act), shown under the title.
    detail?: string;
    /// The file change to review: structured files, or a raw unified diff parsed
    /// client-side. Absent for non-file actions (the card keeps its plain body).
    diff?: DiffFile[] | string;
    /// True once applied: the card becomes the receipt with Undo.
    done?: boolean;
    /// True when applied with no gate, under a standing grant (auto-accept). The
    /// receipt then says so and names the grant, so an unprompted change is never
    /// silent - you see the diff and can undo it, you just did not click Approve.
    auto?: boolean;
    /// The grant that authorised an auto-applied change ("edits in this project").
    via?: string;
    onapprove?: () => void;
    ondeny?: () => void;
    /// Create a grant at the chosen scope ("project" narrow, "type" broad).
    onalways?: (scope: "project" | "type") => void;
    /// Reverse the applied change (the executor compensate).
    onundo?: () => void;
  } = $props();

  const files = $derived(typeof diff === "string" ? parseUnifiedDiff(diff) : (diff ?? []));
  const totals = $derived(diffTotals(files));
</script>

<div class="gate" class:done role="group" aria-label={$t("h.gate.aria")}>
  <div class="gate-head">
    {#if done}
      <CircleCheck size={15} strokeWidth={2} />
    {:else}
      <TriangleAlert size={15} strokeWidth={2} />
    {/if}
    <span class="gate-title">{title}</span>
    {#if done && auto}
      <span class="auto-tag" title={via ? `Allowed by: ${via}` : undefined}>
        <Sparkles size={11} strokeWidth={2} />
        Auto
      </span>
    {/if}
  </div>
  {#if detail}
    <p class="gate-detail">{detail}</p>
  {/if}

  {#if files.length > 0}
    {#if files.length > 1}
      <div class="diff-summary">
        {files.length} files
        <span class="add">+{totals.additions}</span>
        <span class="del">-{totals.deletions}</span>
      </div>
    {/if}
    <DiffView {files} collapsed={done} />
  {/if}

  <div class="gate-actions">
    {#if done}
      <span class="done-note">
        {auto ? "Applied automatically" : "Applied"}
        {#if via}<span class="via">via {via}</span>{/if}
      </span>
      <Button variant="outline" size="sm" onclick={() => onundo?.()}>
        <Undo2 size={13} strokeWidth={2} />
        Undo
      </Button>
    {:else}
      <Button variant="default" size="sm" onclick={() => onapprove?.()}>{$t("h.gate.approve")}</Button>
      {#if onalways}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button variant="outline" size="sm" {...props}>
                Always allow
                <ChevronDown size={13} strokeWidth={2} />
              </Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content class="w-56">
            <DropdownMenu.Item onclick={() => onalways?.("project")}>
              Only in this project
            </DropdownMenu.Item>
            <DropdownMenu.Item onclick={() => onalways?.("type")}>
              This action type generally
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {/if}
      <Button variant="ghost" size="sm" onclick={() => ondeny?.()}>{$t("h.gate.deny")}</Button>
    {/if}
  </div>
</div>

<style>
  .gate {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem var(--space-card, 1rem);
    border: 1px solid color-mix(in srgb, var(--color-warning, #d4b483) 40%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-warning, #d4b483) 8%, transparent);
  }
  /* The applied receipt drops the warning register for a quiet confirmed one. */
  .gate.done {
    border-color: color-mix(in srgb, #8fae74 35%, transparent);
    background: color-mix(in srgb, #8fae74 6%, transparent);
  }
  .gate-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--color-warning, #d4b483);
  }
  .gate.done .gate-head {
    color: #8fae74;
  }
  .gate-head :global(svg) {
    flex-shrink: 0;
  }
  .gate-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .gate-detail {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .diff-summary {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .diff-summary .add {
    color: #8fae74;
    font-variant-numeric: tabular-nums;
  }
  .diff-summary .del {
    color: #c96a6a;
    font-variant-numeric: tabular-nums;
  }
  .gate-actions {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    margin-top: 0.125rem;
  }
  .done-note {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs);
    color: #8fae74;
    margin-right: auto;
  }
  .done-note .via {
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  /* The "Auto" tag marks a change applied with no gate, under a standing grant,
     so an unprompted edit reads as deliberate (your rule), not silent. */
  .auto-tag {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    margin-left: 0.125rem;
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, #8fae74 16%, transparent);
    color: #8fae74;
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
</style>
