<script lang="ts">
  /// Conversation surface (ai-app.md §2.1) — the full GUI door for
  /// human-initiated, multi-turn conversation against the ai-daemon
  /// query path.
  ///
  /// Real round-trips through the `ai_query` command (submit → poll →
  /// answer), plain-text bubbles, a pending state, honest error rendering,
  /// the always-visible capability context, and (A3) the visible tool calls
  /// the daemon made while answering, as collapsible cards. Graph-data
  /// citations and token streaming come later.
  import { tick, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { MessageSquare, ArrowUp, AlertCircle, Eye, Wand2, Wrench, Cpu } from "@lucide/svelte";
  import { messages, busy, send } from "$lib/stores/conversation";

  interface Capability {
    enabled: boolean;
    tier: string;
    actionMode: string;
    provider?: string | null;
    model?: string | null;
  }

  let draft = $state("");
  let scrollEl = $state<HTMLDivElement | null>(null);
  let capability = $state<Capability | null>(null);

  // Always-visible capability context (ai-app.md §2.1): the read tier
  // and action mode the AI operates under, from ai.toml (what the daemon
  // enforces). Refreshed each mount so a Settings change is reflected.
  onMount(async () => {
    try {
      capability = await invoke<Capability>("ai_capability");
    } catch {
      capability = null;
    }
  });

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
  {#if capability}
    <div class="context-bar" title="What the assistant can see and do, from your AI settings">
      <span class="cap" class:off={!capability.enabled}>
        <span class="cap-dot"></span>
        {capability.enabled ? "Enabled" : "Disabled"}
      </span>
      <span class="cap-sep">·</span>
      <span class="cap"><Eye size={12} strokeWidth={1.75} />Reads: {capability.tier}</span>
      <span class="cap-sep">·</span>
      <span class="cap"><Wand2 size={12} strokeWidth={1.75} />{capability.actionMode}</span>
      {#if capability.provider || capability.model}
        <span class="cap-sep">·</span>
        <span class="cap" title="The configured AI provider and model">
          <Cpu size={12} strokeWidth={1.75} />
          {[capability.provider, capability.model].filter(Boolean).join(" · ")}
        </span>
      {/if}
    </div>
  {/if}
  <div class="messages" bind:this={scrollEl}>
    {#if $messages.length === 0}
      <div class="empty-state">
        <MessageSquare size={28} strokeWidth={1.5} />
        <p class="empty-title">Ask the assistant</p>
        <p class="empty-sub">
          Ask the on-device AI about your files, projects, and activity.
          Answers are grounded in your Knowledge Graph under the configured
          read tier. Each question is answered on its own for now —
          conversation memory comes later.
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
              <div class="msg-body">
                {#if msg.toolCalls && msg.toolCalls.length > 0}
                  <div class="tool-calls">
                    {#each msg.toolCalls ?? [] as call, i (i)}
                      <details class="tool-call">
                        <summary>
                          <Wrench size={11} strokeWidth={2} />
                          <span class="tc-name">{call.server}/{call.tool}</span>
                        </summary>
                        <div class="tc-detail">
                          {#if call.arguments}
                            <div class="tc-section">
                              <span class="tc-label">arguments</span>
                              <pre>{call.arguments}</pre>
                            </div>
                          {/if}
                          {#if call.result}
                            <div class="tc-section">
                              <span class="tc-label">result</span>
                              <pre>{call.result}</pre>
                            </div>
                          {/if}
                        </div>
                      </details>
                    {/each}
                  </div>
                {:else if msg.traceUnavailable}
                  <p class="trace-note">Tool trace unavailable for this turn.</p>
                {/if}
                <div class="bubble bubble-{msg.role}">{msg.text}</div>
              </div>
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
  {#if $messages.length > 0}
    <p class="turn-note">Each question is answered independently — no conversation memory yet.</p>
  {/if}
</div>

<style>
  .conversation {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .context-bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 1rem;
    border-bottom: 1px solid var(--color-border);
    font-size: 0.72rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .cap {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }
  .cap-sep {
    opacity: 0.4;
  }
  .cap-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-success);
  }
  .cap.off .cap-dot {
    background: color-mix(in srgb, var(--foreground) 35%, transparent);
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
  .msg-body {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-width: 80%;
  }
  .tool-calls {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .tool-call {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-bg-card) 60%, transparent);
    font-size: 0.78rem;
  }
  .tool-call summary {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.5rem;
    cursor: pointer;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    list-style: none;
  }
  .tool-call summary::-webkit-details-marker {
    display: none;
  }
  .tc-name {
    font-family: var(--font-mono, monospace);
  }
  .tc-detail {
    padding: 0 0.5rem 0.4rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .tc-section {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .tc-label {
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .trace-note {
    margin: 0;
    font-size: 0.7rem;
    font-style: italic;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .tc-detail pre {
    margin: 0;
    padding: 0.35rem 0.45rem;
    background: var(--color-bg-card);
    border-radius: var(--radius-chip);
    font-size: 0.72rem;
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-x: auto;
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
  .turn-note {
    margin: 0;
    padding: 0 1rem 0.5rem;
    font-size: 0.7rem;
    text-align: center;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
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
