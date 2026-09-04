<script lang="ts">
  /// The in-conversation agent tray: pending proposals the user must gate, and
  /// the recent applied-action receipts they can undo, surfaced right above the
  /// composer. The autonomous agent is event-triggered, so this is not tied to a
  /// chat turn - it sits beside the input, impossible to miss. Empty -> renders
  /// nothing. Reuses the GateCard (gate when pending, receipt when done).
  import { t } from "$lib/i18n/messages";
  import GateCard from "./GateCard.svelte";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
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
  const recent = $derived(($completedActions ?? []).slice(0, RECEIPT_CAP));
  const pending = $derived($pendingProposals ?? []);

  /// Null on either list means the read was refused or never answered - a
  /// different fact from "nothing pending", so it gets its own line instead
  /// of the tray silently not rendering.
  const unreadable = $derived($pendingProposals === null || $completedActions === null);

  /// A message KEY, so the sentence re-renders in the current language.
  let notice = $state<string | null>(null);

  /// The proposal's concrete effect(s) + why, under the summary title.
  function proposalDetail(p: PendingProposal): string {
    const what = p.effects?.length ? p.effects.join("; ") : "";
    return [what, p.reason].filter(Boolean).join(", ");
  }

  /// A content diff to review, when the change carries one (moves/graph writes
  /// do not, so the card keeps its plain body).
  function changeDiff(change: PendingProposal["change"] | CompletedAction["change"]): string | undefined {
    return change?.diff;
  }

  function humanStatus(status: string): string {
    if (status.startsWith("not-enabled")) return "h.aa.notEnabled";
    if (status === "no-such-proposal" || status === "no-such-receipt") return "h.aa.gone";
    return "h.aa.failed";
  }

  /// The proposals and receipts whose ask is on the wire right now. Approve,
  /// Deny and Undo are the irreversible acts on this surface, so a second
  /// click while the first is answering must not send a second ask; the
  /// card's buttons are disabled for exactly that window.
  let inflight = $state(new Set<string | number>());

  async function run(key: string | number, fn: () => Promise<string>, ok: string[]) {
    if (inflight.has(key)) return;
    inflight = new Set([...inflight, key]);
    notice = null;
    try {
      const status = await fn();
      if (!ok.includes(status)) notice = humanStatus(status);
    } catch {
      notice = "h.aa.unreachable";
    } finally {
      const next = new Set(inflight);
      next.delete(key);
      inflight = next;
    }
  }

  const approve = (id: number) => run(id, () => approveProposal(id), ["executed", "nothing-to-execute"]);
  const deny = (id: number) => run(id, () => denyProposal(id), ["denied"]);
  const undo = (id: string) => run(id, () => undoAction(id), ["retracted", "nothing-to-undo"]);
</script>

{#if unreadable || pending.length > 0 || recent.length > 0}
  <div class="agent-actions" role="region" aria-label={$t("h.agentActions.aria")}>
    {#if unreadable}
      <Notice tone="caution" text={$t("h.aa.unreadable")} />
    {/if}
    {#if notice}
      <Notice tone="error" text={$t(notice)} />
    {/if}

    {#each pending as p (p.id)}
      <GateCard
        title={p.summary}
        detail={proposalDetail(p)}
        diff={changeDiff(p.change)}
        busy={inflight.has(p.id)}
        onapprove={() => approve(p.id)}
        ondeny={() => deny(p.id)}
      />
    {/each}

    {#each recent as c (c.id)}
      <GateCard title={c.what} diff={changeDiff(c.change)} done busy={inflight.has(c.id)} onundo={() => undo(c.id)} />
    {/each}

    {#if ($completedActions ?? []).length > recent.length}
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
  .aa-all {
    align-self: flex-start;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    text-decoration: none;
  }
  .aa-all:hover {
    color: var(--foreground);
    text-decoration: underline;
  }
</style>
