<script lang="ts">
  /// One turn in the flat document thread: a quiet role line, then the
  /// content. Your turns are bare prose; assistant turns sit on a subtle
  /// tint; errors on the error tint. Tool calls render above the answer,
  /// full width. Actions appear on hover in the gap below the turn: copy,
  /// bookmark, edit (your turns), branch, delete, and try-again on the last
  /// answer. All of them call the existing session store actions.
  import { tick } from "svelte";
  import {
    AlertCircle,
    Bookmark,
    Check,
    Copy,
    GitBranch,
    Paperclip,
    Pencil,
    RotateCcw,
    Trash2,
  } from "@lucide/svelte";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import ToolCallCard from "./ToolCallCard.svelte";
  import ArtifactBlock from "./ArtifactBlock.svelte";
  import ArtifactCard from "./ArtifactCard.svelte";
  import { placement } from "$lib/components/artifact/placement";
  import { renderMarkdown } from "$lib/markdown";
  import { externalLinks } from "$lib/externalLinks";
  import { fileRefs } from "$lib/fileRefs";
  import {
    busy,
    deleteTurn,
    editAndResend,
    fork,
    regenerate,
    togglePin,
    type Message,
  } from "$lib/stores/conversation";
  import { planRegenerate } from "$lib/regenerate";
  import { messages } from "$lib/stores/conversation";

  let {
    message,
    aiReady,
  }: {
    message: Message;
    /// AI confirmed usable; actions that re-ask the assistant require it.
    aiReady: boolean;
  } = $props();

  const isLast = $derived($messages[$messages.length - 1]?.id === message.id);
  const canRegenerate = $derived(
    isLast && aiReady && planRegenerate($messages) !== null,
  );
  // Editing re-sends, so it needs the assistant; attachment turns are not
  // editable (the plan refuses them, mirrored here for the affordance).
  const canEdit = $derived(
    aiReady && message.role === "user" && !message.mentions?.length && !$busy,
  );

  let copied = $state(false);
  async function copyText() {
    if (!message.text) return;
    try {
      await navigator.clipboard.writeText(message.text);
      copied = true;
      setTimeout(() => (copied = false), 1200);
    } catch {
      // Clipboard unavailable; nothing to surface.
    }
  }

  // Inline edit of a user turn: the prose swaps for a textarea; Enter
  // re-sends, Escape cancels.
  let editing = $state(false);
  let editDraft = $state("");
  async function beginEdit() {
    editDraft = message.text;
    editing = true;
    await tick();
  }
  function commitEdit() {
    const text = editDraft.trim();
    editing = false;
    if (text.length === 0 || text === message.text) return;
    editAndResend(message.id, text);
  }
  function onEditKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      commitEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      editing = false;
    }
  }

  async function doRegenerate() {
    if (!canRegenerate || $busy) return;
    await regenerate();
  }
</script>

<div class="turn" data-role={message.role}>
  <p class="role">{message.role === "user" ? "You" : "Assistant"}</p>

  {#if message.pending}
    <div class="block tinted">
      <span class="dots" aria-label="Thinking">
        <span></span><span></span><span></span>
      </span>
    </div>
  {:else if message.role === "error"}
    <div class="block error-block">
      <AlertCircle size={14} strokeWidth={2} />
      <span class="error-text">
        The assistant could not answer. <code>{message.text}</code>
      </span>
      {#if canRegenerate}
        <Button variant="outline" size="sm" class="self-center" disabled={$busy} onclick={doRegenerate}>
          Try again
        </Button>
      {/if}
    </div>
  {:else}
    {#if message.toolCalls && message.toolCalls.length > 0}
      <div class="tools">
        {#each message.toolCalls as call, i (i)}
          <ToolCallCard {call} />
        {/each}
      </div>
    {:else if message.traceUnavailable}
      <p class="trace-note">No details were recorded for this answer.</p>
    {/if}

    {#if editing}
      <div class="edit">
        <Textarea
          bind:value={editDraft}
          rows={1}
          maxRows={8}
          aria-label="Edit your message"
          onkeydown={onEditKeydown}
          {@attach (el: HTMLTextAreaElement) => {
            el.focus();
            el.setSelectionRange(el.value.length, el.value.length);
          }}
        />
        <div class="edit-actions">
          <Button variant="ghost" size="sm" onclick={() => (editing = false)}>Cancel</Button>
          <Button variant="default" size="sm" onclick={commitEdit}>Send again</Button>
        </div>
      </div>
    {:else if message.text}
      {#if message.role === "assistant"}
        <!-- Assistant answers are plain full-width prose (the agent's voice),
             markdown parsed + sanitized (DOMPurify) before this {@html}.
             `fileRefs` upgrades the agent's file-reference anchors into pills. -->
        <div
          class="block prose markdown"
          use:externalLinks
          use:fileRefs={message.text}
        >
          {@html renderMarkdown(message.text)}
        </div>
      {:else}
        <!-- Your turn is the subtly differentiated block, not a bubble. -->
        <div class="block you-block">{message.text}</div>
      {/if}
    {/if}

    {#if message.mentions && message.mentions.length > 0}
      <div class="mentions">
        {#each message.mentions as name (name)}
          <span class="mention"><Paperclip size={11} strokeWidth={2} />{name}</span>
        {/each}
      </div>
    {/if}

    {#if message.artifacts && message.artifacts.length > 0}
      <div class="artifacts">
        {#each message.artifacts as art, i (i)}
          {#if placement(art) === "pane"}
            <ArtifactCard artifact={art} />
          {:else}
            <ArtifactBlock artifact={art} />
          {/if}
        {/each}
      </div>
    {/if}
  {/if}

  {#if !message.pending && !editing}
    <div class="actions">
      {#if message.text}
        <IconAction label="Copy" onclick={copyText}>
          {#if copied}<Check size={13} strokeWidth={2} />{:else}<Copy size={13} strokeWidth={2} />{/if}
        </IconAction>
      {/if}
      <IconAction
        label={message.pinned ? "Remove bookmark" : "Bookmark"}
        active={message.pinned}
        onclick={() => togglePin(message.id)}
      >
        <Bookmark size={13} strokeWidth={2} fill={message.pinned ? "currentColor" : "none"} />
      </IconAction>
      {#if canEdit}
        <IconAction label="Edit and send again" onclick={beginEdit}>
          <Pencil size={13} strokeWidth={2} />
        </IconAction>
      {/if}
      <IconAction
        label="Branch into a new chat from here"
        disabled={$busy}
        onclick={() => fork(message.id)}
      >
        <GitBranch size={13} strokeWidth={2} />
      </IconAction>
      {#if canRegenerate}
        <IconAction label="Try again" disabled={$busy} onclick={doRegenerate}>
          <RotateCcw size={13} strokeWidth={2} />
        </IconAction>
      {/if}
      <IconAction label="Delete this turn" disabled={$busy} onclick={() => deleteTurn(message.id)}>
        <Trash2 size={13} strokeWidth={2} />
      </IconAction>
    </div>
  {/if}
</div>

<style>
  .turn {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  /* Quiet sentence-case role line on the shared text edge. Deliberately not
     an uppercase eyebrow. */
  .role {
    margin: 0;
    padding-inline: var(--space-card, 1rem);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .block {
    font-size: 0.875rem;
    line-height: 1.6;
    color: var(--foreground);
    word-break: break-word;
  }
  /* Assistant prose flows full-width on the column edge, text inset to the
     shared 1rem text edge - no box, the agent's voice. */
  .prose {
    padding-inline: var(--space-card, 1rem);
  }
  /* Your turn: the subtly tinted block (the differentiation, not a bubble). */
  .you-block {
    padding: 0.625rem var(--space-card, 1rem);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    white-space: pre-wrap;
  }
  /* The pending placeholder keeps a faint tint while the agent works. */
  .tinted {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border-radius: var(--radius-card);
    padding: 0.75rem var(--space-card, 1rem);
  }
  .error-block {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    border-radius: var(--radius-card);
    padding: 0.75rem var(--space-card, 1rem);
    color: var(--color-error);
    font-size: 0.8125rem;
    line-height: 1.5;
  }
  .error-block :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
  }
  .error-text {
    min-width: 0;
    flex: 1;
  }
  /* The raw reason is recorded data; the code chip separates it from the
     app's own sentence. */
  .error-text code {
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    padding: 0.1em 0.3em;
    border-radius: var(--radius-chip);
    word-break: break-word;
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .trace-note {
    margin: 0;
    padding-inline: var(--space-card, 1rem);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .mentions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    padding-inline: var(--space-card, 1rem);
  }
  .artifacts {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    margin-top: 0.5rem;
  }
  .mention {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }
  /* Hover actions live in the turn gap below, absolutely positioned so the
     layout never shifts. */
  .actions {
    position: absolute;
    top: calc(100% + 0.125rem);
    left: var(--space-card, 1rem);
    display: flex;
    gap: 0.25rem;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
    z-index: 2;
  }
  .turn:hover .actions,
  .actions:focus-within {
    opacity: 1;
  }
  .dots {
    display: inline-flex;
    gap: 3px;
  }
  .dots span {
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
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
