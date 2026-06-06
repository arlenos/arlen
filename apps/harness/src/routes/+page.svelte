<script lang="ts">
  /// Conversation surface (ai-app.md §2.1) — the full GUI door for
  /// human-initiated, multi-turn conversation against the ai-daemon
  /// query path.
  ///
  /// A2 MVP: real round-trips through the `ai_query` command (submit →
  /// poll → answer), plain-text message bubbles, a pending state, and
  /// honest error rendering. Visible tool calls, graph-data citations,
  /// streaming, and the always-visible capability context come in A3.
  import { tick } from "svelte";
  import { Input } from "@lunaris/ui-kit/components/ui/input";
  import { Button } from "@lunaris/ui-kit/components/ui/button";
  import { MessageSquare, ArrowUp, AlertCircle } from "@lucide/svelte";
  import { messages, busy, send } from "$lib/stores/conversation";

  let draft = $state("");
  let scrollEl = $state<HTMLDivElement | null>(null);

  function scrollToBottom() {
    scrollEl?.scrollTo({ top: scrollEl.scrollHeight, behavior: "smooth" });
  }

  async function submit() {
    const text = draft.trim();
    if (!text || $busy) return;
    draft = "";
    const turn = send(text); // pushes user + pending synchronously
    await tick();
    scrollToBottom();
    await turn;
    await tick();
    scrollToBottom();
  }

  function onKeydown(e: KeyboardEvent) {
    // Enter sends; Shift+Enter is a newline (for future multiline).
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }
</script>

<div class="conversation">
  <div class="messages" bind:this={scrollEl}>
    {#if $messages.length === 0}
      <div class="empty-state">
        <MessageSquare size={28} strokeWidth={1.5} />
        <p class="empty-title">Ask the assistant</p>
        <p class="empty-sub">
          Multi-turn conversation against the on-device AI. Answers are
          grounded in your Knowledge Graph under the configured read tier.
        </p>
      </div>
    {:else}
      <div class="thread">
        {#each $messages as msg (msg.id)}
          <div class="msg msg-{msg.role}">
            {#if msg.role === "error"}
              <div class="bubble bubble-error">
                <AlertCircle size={14} strokeWidth={2} />
                <span>{msg.text}</span>
              </div>
            {:else if msg.pending}
              <div class="bubble bubble-assistant">
                <span class="dots" aria-label="Thinking">
                  <span></span><span></span><span></span>
                </span>
              </div>
            {:else}
              <div class="bubble bubble-{msg.role}">{msg.text}</div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="composer">
    <Input
      bind:value={draft}
      onkeydown={onKeydown}
      placeholder="Ask about your files, projects, activity…"
      disabled={$busy}
      aria-label="Message"
    />
    <Button size="icon" variant="default" onclick={submit} disabled={$busy || draft.trim() === ""} aria-label="Send">
      <ArrowUp size={16} strokeWidth={2} />
    </Button>
  </div>
</div>

<style>
  .conversation {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .messages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 1.5rem;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
    gap: 0.5rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .empty-title {
    margin: 0.25rem 0 0;
    font-size: 1rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .empty-sub {
    margin: 0;
    max-width: 26rem;
    font-size: 0.85rem;
    line-height: 1.5;
  }
  .thread {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 48rem;
    margin-inline: auto;
  }
  .msg {
    display: flex;
  }
  .msg-user {
    justify-content: flex-end;
  }
  .msg-assistant,
  .msg-error {
    justify-content: flex-start;
  }
  .bubble {
    max-width: 80%;
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-card);
    font-size: 0.875rem;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .bubble-user {
    background: var(--color-accent);
    color: var(--color-accent-foreground);
    border-bottom-right-radius: var(--radius-chip);
  }
  .bubble-assistant {
    background: var(--color-bg-card);
    color: var(--foreground);
    border: 1px solid var(--color-border);
    border-bottom-left-radius: var(--radius-chip);
  }
  .bubble-error {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-error) 30%, transparent);
    color: var(--color-error);
  }
  .composer {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-top: 1px solid var(--color-border);
  }
  .composer :global(input) {
    flex: 1;
  }
  .dots {
    display: inline-flex;
    gap: 3px;
  }
  .dots span {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--foreground) 50%, transparent);
    animation: dot 1.2s infinite ease-in-out;
  }
  .dots span:nth-child(2) {
    animation-delay: 0.15s;
  }
  .dots span:nth-child(3) {
    animation-delay: 0.3s;
  }
  @keyframes dot {
    0%, 60%, 100% {
      opacity: 0.3;
      transform: translateY(0);
    }
    30% {
      opacity: 1;
      transform: translateY(-2px);
    }
  }
</style>
