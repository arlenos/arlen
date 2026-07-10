<script lang="ts">
  /// The in-conversation agent tray: pending proposals the user must gate, and
  /// the recent applied-action receipts they can undo, surfaced right above the
  /// composer. The autonomous agent is event-triggered, so this is not tied to a
  /// chat turn - it sits beside the input, impossible to miss. Empty -> renders
  /// nothing. Reuses the GateCard (gate when pending, receipt when done).
  import { t } from "$lib/i18n/messages";
  import GateCard from "./GateCard.svelte";
  import {
    pendingProposals,
    completedActions,
    approveProposal,
    denyProposal,
    undoAction,
    type PendingProposal,
    type CompletedAction,
  } from "$lib/stores/agentActions";

  /// Show only the most recent receipts inline; the full history is on /agent.
  const RECEIPT_CAP = 4;
  const recent = $derived($completedActions.slice(0, RECEIPT_CAP));

  let notice = $state<string | null>(null);

  /// The proposal's concrete effect(s) + why, under the summary title.
  function proposalDetail(p: PendingProposal): string {
    const what = p.effects?.length ? p.effects.join("; ") : "";
    return [what, p.reason].filter(Boolean).join(" · ");
  }

  /// A content diff to review, when the change carries one (moves/graph writes
  /// do not, so the card keeps its plain body).
  function changeDiff(change: PendingProposal["change"] | CompletedAction["change"]): string | undefined {
    return change?.diff;
  }

  function humanStatus(status: string): string {
    if (status.startsWith("not-enabled")) return "Turn on live actions in the composer to apply this.";
    if (status === "no-such-proposal" || status === "no-such-receipt")
      return "That action is no longer available.";
    return "Something went wrong reaching the agent.";
  }

  async function run(fn: () => Promise<string>, ok: string[]) {
    notice = null;
    try {
      const status = await fn();
      if (!ok.includes(status)) notice = humanStatus(status);
    } catch {
      notice = "Could not reach the agent.";
    }
  }

  const approve = (id: number) => run(() => approveProposal(id), ["executed", "nothing-to-execute"]);
  const deny = (id: number) => run(() => denyProposal(id), ["denied"]);
  const undo = (id: string) => run(() => undoAction(id), ["retracted", "nothing-to-undo"]);
</script>

{#if $pendingProposals.length > 0 || recent.length > 0}
  <div class="agent-actions" role="region" aria-label={$t("h.agentActions.aria")}>
    {#if notice}
      <p class="aa-notice" role="status">{notice}</p>
    {/if}

    {#each $pendingProposals as p (p.id)}
      <GateCard
        title={p.summary}
        detail={proposalDetail(p)}
        diff={changeDiff(p.change)}
        onapprove={() => approve(p.id)}
        ondeny={() => deny(p.id)}
      />
    {/each}

    {#each recent as c (c.id)}
      <GateCard title={c.what} diff={changeDiff(c.change)} done onundo={() => undo(c.id)} />
    {/each}

    {#if $completedActions.length > recent.length}
      <a class="aa-all" href="/agent">{$t("h.agentActions.seeAll")}</a>
    {/if}
  </div>
{/if}

<style>
  .agent-actions {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    width: 100%;
    max-width: var(--width-thread, 48rem);
    margin-inline: auto;
    margin-bottom: 0.5rem;
    /* A burst of proposals scrolls within the tray instead of shoving the
       composer off-screen. */
    max-height: 40vh;
    overflow-y: auto;
  }
  .aa-notice {
    margin: 0;
    font-size: 0.75rem;
    color: var(--color-warning, #d4b483);
  }
  .aa-all {
    align-self: flex-start;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    text-decoration: none;
  }
  .aa-all:hover {
    color: var(--foreground);
    text-decoration: underline;
  }
</style>
