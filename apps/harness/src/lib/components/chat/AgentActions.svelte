<script lang="ts">
  /// The in-conversation agent tray: the recent applied-action receipts, each
  /// with an undo, right above the composer. The autonomous agent is
  /// event-triggered, so this is not tied to a chat turn. Empty -> renders
  /// nothing. Reuses the GateCard in its done form.
  import { t } from "$lib/i18n/messages";
  import GateCard from "./GateCard.svelte";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { completedActions, undoAction, type CompletedAction } from "$lib/stores/agentActions";

  /// Show only the most recent receipts inline; the full history is on /agent.
  const RECEIPT_CAP = 4;
  const recent = $derived(($completedActions ?? []).slice(0, RECEIPT_CAP));

  /// Null means the read was refused or never answered - a different fact
  /// from "nothing happened", so it gets its own line instead of the tray
  /// silently not rendering.
  const unreadable = $derived($completedActions === null);

  /// A message KEY, so the sentence re-renders in the current language.
  let notice = $state<string | null>(null);

  /// A content diff to review, when the change carries one (moves and graph
  /// writes do not, so the card keeps its plain body).
  function changeDiff(change: CompletedAction["change"]): string | undefined {
    return change?.diff;
  }

  function humanStatus(status: string): string {
    if (status.startsWith("not-enabled")) return "h.aa.notEnabled";
    if (status === "no-such-receipt") return "h.aa.gone";
    return "h.aa.failed";
  }

  /// The receipts whose undo is on the wire right now. A second click while
  /// the first is answering must not send a second ask; the card's button is
  /// disabled for exactly that window.
  let inflight = $state(new Set<string>());

  async function run(key: string, fn: () => Promise<string>, ok: string[]) {
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

  const undo = (id: string) => run(id, () => undoAction(id), ["retracted", "nothing-to-undo"]);
</script>

{#if unreadable || recent.length > 0}
  <div class="agent-actions" role="region" aria-label={$t("h.agentActions.aria")}>
    {#if unreadable}
      <Notice tone="caution" text={$t("h.aa.unreadable")} />
    {/if}
    {#if notice}
      <Notice tone="error" text={$t(notice)} />
    {/if}

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
