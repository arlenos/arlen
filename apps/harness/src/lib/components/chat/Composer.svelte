<script lang="ts">
  /// The composer: one bordered container holding the attachment chips, the
  /// growing textarea, and a footer with attach and send. Below the
  /// container the page renders the capability line; nothing else.
  ///
  /// Wired in here: per-chat drafts (typing is never lost on a switch),
  /// shell-style prompt recall on the arrow keys, and the `@` file picker
  /// (the paperclip opens the same picker). Enter sends, Shift+Enter breaks
  /// the line.
  import { tick } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { ArrowUp, ChevronDown, File as FileIcon, Folder, Paperclip, ShieldCheck } from "@lucide/svelte";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import type { Capability } from "$lib/capability";
  import { tierBadge } from "$lib/display";
  import { openTransparency } from "$lib/stores/transparency";
  import ContextChips from "./ContextChips.svelte";
  import {
    activeSessionId,
    busy,
    messages,
    send,
    type MentionContent,
  } from "$lib/stores/conversation";
  import { getDraft, setDraft } from "$lib/stores/drafts";
  import { navigateHistory, promptHistory } from "$lib/prompt-history";

  interface FileSuggestion {
    path: string;
    name: string;
    isDir: boolean;
  }

  let {
    disabled,
    placeholder,
    capability,
  }: {
    /// Disable input entirely (AI off or unreachable); the page renders the
    /// matching status line below.
    disabled: boolean;
    placeholder: string;
    /// The capability read, so the foot can carry the quiet posture + model
    /// chip. `null` while loading or after a failed read; the chip then hides.
    capability: Capability | null;
  } = $props();

  // The posture chip shows only when the AI is on; off / unreachable states
  // are carried by the warning line below, not by a foot chip. The dropdown
  // flips how much the agent may act on its own; the choice updates here
  // optimistically and the daemon `ai_set_posture` swap confirms it (until
  // that command lands the choice reverts, so nothing is faked).
  let postureOverride = $state<boolean | null>(null);
  const liveExecutor = $derived(postureOverride ?? capability?.executorLive ?? false);
  const badge = $derived(
    capability?.enabled ? tierBadge({ ...capability, executorLive: liveExecutor }) : null,
  );
  const dialValue = $derived(liveExecutor ? "auto" : "suggest");

  async function applyPosture(value: string) {
    const live = value === "auto";
    if (live === liveExecutor) return;
    postureOverride = live;
    try {
      await invoke("ai_set_posture", { executorLive: live });
    } catch {
      postureOverride = null;
    }
  }

  let draft = $state("");
  let textareaRef = $state<HTMLTextAreaElement | null>(null);

  // Per-chat drafts: restore on switch, persist as the user types. The
  // sent-draft cleanup happens in the store (`send` clears it), so switching
  // back to a chat never resurrects a sent prompt.
  let draftSession = $state<string | null>(null);
  $effect(() => {
    const id = $activeSessionId;
    if (id !== draftSession) {
      draftSession = id;
      draft = id ? getDraft(id) : "";
      hist = { index: null, saved: "" };
      // Switching chats puts the caret where typing continues, like any
      // desktop chat client.
      if (!disabled) textareaRef?.focus();
    }
  });
  function onInput() {
    if (draftSession) setDraft(draftSession, draft);
    // Any edit leaves history-recall mode; the live draft is the new truth.
    hist = { index: null, saved: "" };
  }

  /// Insert a starter or programmatic text (also used by the page).
  export function setText(text: string) {
    draft = text;
    if (draftSession) setDraft(draftSession, draft);
    textareaRef?.focus();
  }

  // Shell-style prompt recall. Only the arrow keys at the caret boundaries
  // navigate, so normal multi-line editing is untouched.
  let hist = $state<{ index: number | null; saved: string }>({ index: null, saved: "" });
  function recallOlder(): boolean {
    const history = promptHistory($messages);
    if (history.length === 0) return false;
    if (hist.index === null) hist = { index: null, saved: draft };
    const nav = navigateHistory(history, hist.index, "older");
    hist = { ...hist, index: nav.index };
    draft = nav.text;
    return true;
  }
  function recallNewer(): boolean {
    if (hist.index === null) return false;
    const history = promptHistory($messages);
    const nav = navigateHistory(history, hist.index, "newer");
    draft = nav.index === null ? hist.saved : nav.text;
    hist = { ...hist, index: nav.index };
    return true;
  }

  // `@`-mention state. The picker's contents come from a Tauri call, and the
  // Svelte-5 caveat applies: `$state` mutated from an IPC callback does not
  // reliably re-render, so the picker's reactive data lives in `writable`
  // stores while `draft` (user-driven) stays `$state`.
  const suggestions = writable<FileSuggestion[]>([]);
  const mentionOpen = writable(false);
  const mentionIndex = writable(0);
  const attached = writable<MentionContent[]>([]);
  let mentionAt = -1;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  /// The active `@`-mention being typed: the run from the last `@` to the end
  /// of the draft, but only when that `@` sits at a word boundary and the run
  /// holds no whitespace.
  function activeMention(s: string): { at: number; query: string } | null {
    const at = s.lastIndexOf("@");
    if (at === -1) return null;
    if (at > 0 && !/\s/.test(s[at - 1])) return null;
    const query = s.slice(at + 1);
    if (/\s/.test(query)) return null;
    return { at, query };
  }

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
      // Descend: keep the picker open and list that directory next.
      draft = draft.slice(0, mentionAt) + "@" + s.path + "/";
      return;
    }
    try {
      const content = await invoke<MentionContent>("read_mention_file", { path: s.path });
      attached.update((list) =>
        list.some((m) => m.path === content.path) ? list : [...list, content],
      );
    } catch {
      // Attach failed; the draft keeps the typed token so nothing is lost.
      return;
    }
    draft = draft.slice(0, mentionAt);
    if (draftSession) setDraft(draftSession, draft);
    closeMention();
  }

  function removeAttached(path: string) {
    attached.update((list) => list.filter((m) => m.path !== path));
  }

  /// The paperclip opens the same picker the `@` key does.
  async function openPicker() {
    if (disabled || $busy) return;
    const needsSpace = draft.length > 0 && !/\s$/.test(draft);
    draft = draft + (needsSpace ? " @" : "@");
    onInput();
    await tick();
    textareaRef?.focus();
  }

  async function submit() {
    const text = draft.trim();
    const mentions = $attached;
    if ((!text && mentions.length === 0) || $busy || disabled) return;
    draft = "";
    attached.set([]);
    closeMention();
    hist = { index: null, saved: "" };
    await send(text, mentions);
  }

  function onKeydown(e: KeyboardEvent) {
    // While the picker is open, the arrows, Enter and Escape drive it.
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
    // Prompt recall: up at the very start, down at the very end while
    // recalling.
    const el = e.currentTarget as HTMLTextAreaElement;
    if (e.key === "ArrowUp" && el.selectionStart === 0 && el.selectionEnd === 0) {
      if (recallOlder()) e.preventDefault();
      return;
    }
    if (e.key === "ArrowDown" && hist.index !== null && el.selectionEnd === el.value.length) {
      if (recallNewer()) e.preventDefault();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }
</script>

<div class="composer-zone">
  {#if $mentionOpen}
    <div class="mention-popover" role="listbox" aria-label="File suggestions">
      {#each $suggestions as s, i (s.path)}
        <button
          type="button"
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

  <div class="composer" class:disabled>
    <ContextChips attached={$attached} onremove={removeAttached} />
    <Textarea
      bind:ref={textareaRef}
      bind:value={draft}
      rows={1}
      maxRows={8}
      class="composer-input"
      {placeholder}
      disabled={disabled || $busy}
      aria-label="Message"
      oninput={onInput}
      onkeydown={onKeydown}
    />
    <div class="composer-foot">
      <div class="foot-left">
        <IconAction label="Attach a file" size="control" disabled={disabled || $busy} onclick={openPicker}>
          <Paperclip size={14} strokeWidth={2} />
        </IconAction>
        <IconAction label="Transparency: what the assistant can reach, read, and did" size="control" onclick={() => openTransparency()}>
          <ShieldCheck size={14} strokeWidth={2} />
        </IconAction>
      </div>
      <div class="foot-right">
        {#if badge}
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props })}
                <button
                  type="button"
                  class="posture"
                  aria-label="How the assistant acts"
                  {...props}
                >
                  <span class="posture-glyph" aria-hidden="true">{badge.glyph}</span>
                  <span class="posture-label">{badge.label}</span>
                  <ChevronDown size={12} strokeWidth={2} class="posture-chev" />
                </button>
              {/snippet}
            </DropdownMenu.Trigger>
            <DropdownMenu.Content side="top" align="end" class="posture-menu">
              <DropdownMenu.Label>How the assistant acts</DropdownMenu.Label>
              <DropdownMenu.RadioGroup value={dialValue} onValueChange={applyPosture}>
                <DropdownMenu.RadioItem value="suggest">
                  Suggests only, I approve each action
                </DropdownMenu.RadioItem>
                <DropdownMenu.RadioItem value="auto">
                  Acts on small things, I can undo them
                </DropdownMenu.RadioItem>
              </DropdownMenu.RadioGroup>
              <DropdownMenu.Separator />
              <DropdownMenu.Item onclick={() => openTransparency()}>
                Manage in Transparency
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        {/if}
        <Button
          size="icon-sm"
          variant="default"
          aria-label="Send"
          disabled={disabled || $busy || (draft.trim() === "" && $attached.length === 0)}
          onclick={submit}
        >
          <ArrowUp size={16} strokeWidth={2} />
        </Button>
      </div>
    </div>
  </div>
</div>

<style>
  .composer-zone {
    position: relative;
    max-width: var(--width-thread, 48rem);
    width: 100%;
    margin-inline: auto;
    padding-inline: var(--space-page, 1.5rem);
  }
  .composer {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    background: var(--color-bg-input);
    transition: border-color var(--duration-fast) var(--ease-out);
  }
  .composer:focus-within {
    border-color: var(--color-border-strong);
  }
  .composer.disabled {
    opacity: 0.7;
  }
  /* The textarea is borderless inside the container; the container is the
     control. */
  .composer :global(.composer-input) {
    border: none;
    background: transparent;
    padding: 0.75rem var(--space-card, 1rem) 0.25rem;
    border-radius: var(--radius-card) var(--radius-card) 0 0;
  }
  .composer :global(.composer-input:focus-visible) {
    outline: none;
    box-shadow: none;
    --tw-ring-color: transparent;
  }
  .composer-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 0.5rem 0.5rem;
  }
  .foot-left {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    min-width: 0;
  }
  .foot-right {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  /* The posture chip: a quiet, glanceable read of how much the agent may act
     on its own, left of Send. Click opens the dropdown to change it. */
  .posture {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
    height: var(--height-control, 28px);
    padding: 0 0.5rem;
    border: none;
    border-radius: var(--radius-button);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-size: 0.75rem;
    transition: color var(--duration-fast) var(--ease-out);
  }
  .posture:hover {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .posture-glyph {
    color: var(--color-success);
  }
  .posture-label {
    flex-shrink: 0;
  }
  :global(.posture-chev) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .mention-popover {
    position: absolute;
    bottom: 100%;
    left: var(--space-page, 1.5rem);
    right: var(--space-page, 1.5rem);
    max-height: 14rem;
    overflow-y: auto;
    margin-bottom: 0.5rem;
    padding: 0.25rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    /* The items hug the inside; their corners follow this radius minus the
       0.25rem padding (rounding-fix.md). */
    --container-radius: var(--radius-card);
    --container-inset: 0.25rem;
    z-index: 20;
  }
  .mention-item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    min-height: var(--height-control, 28px);
    padding: 0 0.5rem;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.8125rem;
    text-align: left;
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
  }
  .mention-item.active {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
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
</style>
