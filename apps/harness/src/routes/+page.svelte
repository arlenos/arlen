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
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { MessageSquare, ArrowUp, AlertCircle, Eye, Wand2, Wrench, Cpu, File as FileIcon, Folder, Paperclip, X, Copy, Check } from "@lucide/svelte";
  import { messages, busy, send, initSessions, type MentionContent } from "$lib/stores/conversation";
  import ConversationRail from "$lib/components/ConversationRail.svelte";
  import { renderMarkdown } from "$lib/markdown";
  import { conversationToMarkdown } from "$lib/export";
  import { externalLinks } from "$lib/externalLinks";

  interface FileSuggestion {
    path: string;
    name: string;
    isDir: boolean;
  }

  interface Capability {
    enabled: boolean;
    tier: string;
    actionMode: string;
    provider?: string | null;
    model?: string | null;
  }

  let draft = $state("");

  // Starter prompts shown on the empty conversation, grounded in what the
  // Knowledge-Graph-backed assistant can actually answer. Clicking one fills
  // the composer (rather than sending immediately) so the user can edit first.
  const STARTERS = [
    "What did I work on today?",
    "Which files are part of my current project?",
    "What is my computer doing right now?",
    "Summarise my recent activity.",
  ];
  function useStarter(text: string) {
    draft = text;
  }
  let scrollEl = $state<HTMLDivElement | null>(null);
  let capability = $state<Capability | null>(null);

  // The message whose copy button was just pressed, so its button can flash a
  // check for a moment. Reset by a timer; `null` when no copy is pending.
  let copiedId = $state<number | null>(null);

  // Copy a message's raw text to the clipboard. Copies the source the user sees
  // (the markdown for an assistant turn, the typed text for a user turn), not
  // the rendered HTML. Fails silently: a copy that does not land is a minor
  // annoyance, not worth an error bubble.
  async function copyMessage(id: number, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copiedId = id;
      setTimeout(() => {
        if (copiedId === id) copiedId = null;
      }, 1200);
    } catch {
      // Clipboard unavailable (locked-down webview); nothing to surface.
    }
  }

  // Whether the whole-conversation copy just fired, to flash the button.
  let convCopied = $state(false);

  // Copy the active conversation to the clipboard as a Markdown transcript.
  // Same fail-silent clipboard handling as a single message.
  async function copyConversation() {
    const md = conversationToMarkdown($messages);
    if (md.length === 0) return;
    try {
      await navigator.clipboard.writeText(md);
      convCopied = true;
      setTimeout(() => {
        convCopied = false;
      }, 1200);
    } catch {
      // Clipboard unavailable; nothing to surface.
    }
  }

  // `@`-mention state. The popover's contents come from a Tauri call, and this
  // codebase's Svelte-5 caveat is that `$state` mutated from an IPC callback
  // does not reliably re-render — so the picker's reactive data lives in
  // `writable` stores (which do render via `$`-subscription), while `draft`
  // (user-driven, bound to the input) stays `$state`.
  const suggestions = writable<FileSuggestion[]>([]);
  const mentionOpen = writable(false);
  const mentionIndex = writable(0);
  // Files the user has attached to the next turn, read and capped backend-side.
  const attached = writable<MentionContent[]>([]);
  // Where in `draft` the active `@token` starts, for replacing it on select.
  let mentionAt = -1;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  /// The active `@`-mention being typed: the run from the last `@` to the end
  /// of the draft, but only when that `@` sits at a word boundary and the run
  /// holds no whitespace (so a completed mention or a stray `@` mid-word does
  /// not reopen the picker). Returns the `@` offset and the query after it.
  function activeMention(s: string): { at: number; query: string } | null {
    const at = s.lastIndexOf("@");
    if (at === -1) return null;
    if (at > 0 && !/\s/.test(s[at - 1])) return null;
    const query = s.slice(at + 1);
    if (/\s/.test(query)) return null;
    return { at, query };
  }

  // Detect the active mention as the draft changes and fetch suggestions,
  // debounced. Reading `draft` (a `$state`) makes this effect re-run on every
  // keystroke; the async results land in the stores above.
  $effect(() => {
    const m = activeMention(draft);
    if (!m) {
      mentionOpen.set(false);
      suggestions.set([]);
      mentionAt = -1;
      return;
    }
    mentionAt = m.at;
    const q = m.query;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(async () => {
      try {
        const res = await invoke<FileSuggestion[]>("list_files", { query: q });
        suggestions.set(res);
        mentionIndex.set(0);
        mentionOpen.set(res.length > 0);
      } catch {
        suggestions.set([]);
        mentionOpen.set(false);
      }
    }, 120);
  });

  function closeMention() {
    mentionOpen.set(false);
    suggestions.set([]);
    mentionAt = -1;
  }

  async function selectSuggestion(s: FileSuggestion) {
    if (mentionAt < 0) return;
    if (s.isDir) {
      // Descend: replace the `@token` with the directory path and a trailing
      // slash, which keeps the picker open and lists that directory next.
      draft = draft.slice(0, mentionAt) + "@" + s.path + "/";
      return;
    }
    // Attach the file: read its (capped) text, add a chip, and strip the
    // `@token` from the draft so the typed message stays clean.
    try {
      const content = await invoke<MentionContent>("read_mention_file", { path: s.path });
      attached.update((list) =>
        list.some((m) => m.path === content.path) ? list : [...list, content],
      );
    } catch (e) {
      console.error("read_mention_file failed", e);
    }
    draft = draft.slice(0, mentionAt);
    closeMention();
  }

  function removeAttached(path: string) {
    attached.update((list) => list.filter((m) => m.path !== path));
  }

  // Always-visible capability context (ai-app.md §2.1): the read tier
  // and action mode the AI operates under, from ai.toml (what the daemon
  // enforces). Refreshed each mount so a Settings change is reflected.
  onMount(async () => {
    // Load persisted conversations so the history rail is populated on open.
    initSessions();
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
    const mentions = $attached;
    if ((!text && mentions.length === 0) || $busy) return;
    draft = "";
    attached.set([]);
    closeMention();
    const turn = send(text, mentions); // pushes user + pending synchronously
    await tick();
    scrollToBottom();
    await turn;
    await tick();
    scrollToBottom();
  }

  function onKeydown(e: KeyboardEvent) {
    // While the `@` picker is open, the arrow keys / Enter / Escape drive it
    // instead of the composer, so a mention is chosen without leaving the input.
    if ($mentionOpen) {
      const list = $suggestions;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        mentionIndex.update((i) => (i + 1) % list.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        mentionIndex.update((i) => (i - 1 + list.length) % list.length);
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const chosen = list[$mentionIndex];
        if (chosen) selectSuggestion(chosen);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closeMention();
        return;
      }
    }
    // Enter sends; Shift+Enter is a newline (for future multiline).
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }
</script>

<div class="chat-shell">
  <ConversationRail />
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
      {#if $messages.length > 0}
        <button
          class="conv-export"
          title="Copy the conversation as Markdown"
          onclick={copyConversation}
        >
          {#if convCopied}
            <Check size={12} strokeWidth={2} />Copied
          {:else}
            <Copy size={12} strokeWidth={2} />Copy chat
          {/if}
        </button>
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
        <div class="starters">
          {#each STARTERS as s (s)}
            <button class="starter" onclick={() => useStarter(s)}>{s}</button>
          {/each}
        </div>
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
                {#if msg.text}
                  {#if msg.role === "assistant"}
                    <!-- Assistant answers are markdown; renderMarkdown parses
                         and sanitizes them (DOMPurify) before this {@html}. -->
                    <div class="bubble bubble-assistant markdown" use:externalLinks>{@html renderMarkdown(msg.text)}</div>
                  {:else}
                    <div class="bubble bubble-{msg.role}">{msg.text}</div>
                  {/if}
                {/if}
                {#if msg.mentions && msg.mentions.length > 0}
                  <div class="msg-mentions">
                    {#each msg.mentions as name (name)}
                      <span class="mention-chip"><Paperclip size={11} strokeWidth={2} />{name}</span>
                    {/each}
                  </div>
                {/if}
                {#if msg.text}
                  <div class="msg-actions">
                    <button
                      class="msg-copy"
                      aria-label="Copy message"
                      title="Copy message"
                      onclick={() => copyMessage(msg.id, msg.text)}
                    >
                      {#if copiedId === msg.id}
                        <Check size={13} strokeWidth={2} /><span>Copied</span>
                      {:else}
                        <Copy size={13} strokeWidth={2} /><span>Copy</span>
                      {/if}
                    </button>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="composer-wrap">
    {#if $attached.length > 0}
      <div class="attached">
        {#each $attached as m (m.path)}
          <span class="attach-chip" title={m.path}>
            <Paperclip size={11} strokeWidth={2} />
            <span class="attach-name">{m.name}{m.truncated ? " (truncated)" : ""}</span>
            <button class="attach-x" onclick={() => removeAttached(m.path)} aria-label={`Remove ${m.name}`}>
              <X size={11} strokeWidth={2.5} />
            </button>
          </span>
        {/each}
      </div>
    {/if}

    {#if $mentionOpen}
      <div class="mention-popover" role="listbox" aria-label="File suggestions">
        {#each $suggestions as s, i (s.path)}
          <button
            class="mention-item"
            class:active={i === $mentionIndex}
            role="option"
            aria-selected={i === $mentionIndex}
            onmouseenter={() => mentionIndex.set(i)}
            onclick={() => selectSuggestion(s)}
          >
            {#if s.isDir}
              <Folder size={13} strokeWidth={1.75} />
            {:else}
              <FileIcon size={13} strokeWidth={1.75} />
            {/if}
            <span class="mention-name">{s.name}{s.isDir ? "/" : ""}</span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="composer">
      <Input
        bind:value={draft}
        onkeydown={onKeydown}
        placeholder="Ask about your files, projects, activity… (@ to attach a file)"
        disabled={$busy}
        aria-label="Message"
      />
      <Button size="icon" variant="default" onclick={submit} disabled={$busy || (draft.trim() === "" && $attached.length === 0)} aria-label="Send">
        <ArrowUp size={16} strokeWidth={2} />
      </Button>
    </div>
  </div>
  {#if $messages.length > 0}
    <p class="turn-note">Each question is answered independently — no conversation memory yet.</p>
  {/if}
  </div>
</div>

<style>
  .chat-shell {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .conversation {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
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
  /* Pushed to the right edge of the context bar. */
  .conv-export {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.15rem 0.45rem;
    border: 1px solid var(--color-border);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    font-size: 0.7rem;
    border-radius: var(--radius-chip);
    cursor: pointer;
    transition: background 0.1s, color 0.1s;
  }
  .conv-export:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
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
  .starters {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 0.4rem;
    max-width: 30rem;
    margin-top: 0.75rem;
  }
  .starter {
    padding: 0.4rem 0.7rem;
    border: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--color-bg-card) 50%, transparent);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font-size: 0.8rem;
    border-radius: var(--radius-chip);
    cursor: pointer;
    transition: background 0.1s, color 0.1s, border-color 0.1s;
  }
  .starter:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
    border-color: color-mix(in srgb, var(--color-accent) 40%, transparent);
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
  /* Per-message copy, revealed on hover of the message (or on keyboard focus),
     aligned to the bubble's side. */
  .msg-actions {
    display: flex;
  }
  .msg-user .msg-actions {
    justify-content: flex-end;
  }
  .msg-copy {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.35rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-size: 0.7rem;
    border-radius: var(--radius-chip);
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .msg:hover .msg-copy,
  .msg-copy:focus-visible {
    opacity: 1;
  }
  .msg-copy:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
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
  /* Markdown element styling (for the sanitized {@html} assistant answers)
     lives globally in app.css, shared with the agent system-explanation. */
  .bubble-error {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-error) 30%, transparent);
    color: var(--color-error);
  }
  .composer-wrap {
    position: relative;
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
  .attached {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.5rem 1rem 0;
  }
  .attach-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.15rem 0.3rem 0.15rem 0.5rem;
    border-radius: var(--radius-chip);
    border: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--color-bg-card) 60%, transparent);
    font-size: 0.75rem;
    max-width: 18rem;
  }
  .attach-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attach-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.1rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    border-radius: 4px;
  }
  .attach-x:hover {
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .mention-popover {
    position: absolute;
    bottom: 100%;
    left: 1rem;
    right: 1rem;
    max-height: 14rem;
    overflow-y: auto;
    margin-bottom: 0.25rem;
    padding: 0.25rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg, 0 10px 30px rgba(0, 0, 0, 0.3));
    z-index: 20;
  }
  .mention-item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.35rem 0.5rem;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.8125rem;
    text-align: left;
    cursor: pointer;
    border-radius: var(--radius-chip);
  }
  .mention-item.active {
    background: color-mix(in srgb, var(--color-accent) 16%, transparent);
  }
  .mention-item :global(svg) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .mention-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .msg-mentions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .mention-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: 0.7rem;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
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
