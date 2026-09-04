<script lang="ts">
  /// A conversation: the subject once, then one block per message in sent
  /// order. Older blocks start folded to their sender line (the newest is the
  /// reason you are here), and every block carries the full trust-and-content
  /// half when open - a folded message never hides a warning silently, so a
  /// block with a notice says so on its folded line.
  import { ChevronDown, TriangleAlert } from "@lucide/svelte";
  import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { Avatar, AvatarFallback } from "@arlen/ui-kit/components/ui/avatar";
  import { locale } from "$lib/i18n/messages";
  import { displayName, formatSent } from "$lib/wording";
  import type { Message } from "$lib/stores/mailbox";
  import MessageBody from "./MessageBody.svelte";

  let { subject, messages }: { subject: string; messages: Message[] } = $props();

  function letterOf(m: Message): string {
    const n = m.from ? displayName(m.from) : "?";
    return (n.trim()[0] ?? "?").toUpperCase();
  }
  function cautions(m: Message): boolean {
    return (
      m.refusal !== null ||
      m.only_in_text.length > 0 ||
      m.only_in_html.length > 0 ||
      m.channels.length > 0
    );
  }
</script>

<article class="thread">
  <h2 class="subject">{subject}</h2>
  {#each messages as m, i (m.path || i)}
    <Collapsible open={i === messages.length - 1} class="block">
      <CollapsibleTrigger class="trigger-line">
        <Avatar class="size-7">
          <AvatarFallback>{letterOf(m)}</AvatarFallback>
        </Avatar>
        <span class="who">{m.from ? displayName(m.from) : "-"}</span>
        {#if cautions(m)}
          <TriangleAlert size={13} strokeWidth={2} class="warn" aria-hidden="true" />
        {/if}
        <span class="when">{m.date ? formatSent(m.date, $locale) : ""}</span>
        <ChevronDown size={14} strokeWidth={2} class="chev" aria-hidden="true" />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div class="block-body">
          <MessageBody message={m} />
        </div>
      </CollapsibleContent>
    </Collapsible>
  {/each}
</article>

<style>
  .thread {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    width: 100%;
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .subject {
    margin: 0 0 0.5rem;
    font-size: var(--text-xl, 19px);
    font-weight: 600;
    line-height: 1.3;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .thread :global(.block) {
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .thread :global(.trigger-line) {
    display: flex;
    width: 100%;
    align-items: center;
    gap: 0.6rem;
    padding: 0.55rem 0.75rem;
    border: none;
    background: transparent;
    text-align: start;
    color: inherit;
    font: inherit;
  }
  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm, 13px);
    font-weight: 500;
  }
  .thread :global(.warn) {
    flex-shrink: 0;
    color: var(--color-warning, #eab308);
  }
  .when {
    flex-shrink: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .thread :global(.chev) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    transition: rotate var(--duration-fast, 150ms) ease;
  }
  .thread :global([data-state="open"]) :global(.chev) {
    rotate: 180deg;
  }
  .block-body {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
    padding: 0.25rem 0.75rem 0.9rem;
  }
</style>
