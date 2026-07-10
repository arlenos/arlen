<script lang="ts">
  /// The flat document message list: owns the scroll region, follows new
  /// messages while the reader is at the bottom, and offers a jump-back
  /// control after scrolling up. Hosts the empty state when there is
  /// nothing to show.
  import { t } from "$lib/i18n/messages";
  import { tick } from "svelte";
  import { ChevronDown, ChevronUp, X } from "@lucide/svelte";
  import ChatMessage from "./ChatMessage.svelte";
  import EmptyState from "./EmptyState.svelte";
  import FileRefMenu from "./FileRefMenu.svelte";
  import { messages } from "$lib/stores/conversation";
  import { jumpToMessage, findOpen } from "$lib/stores/chatNav";
  import { matchingMessages } from "$lib/search";

  let {
    emptyVariant,
    showEmpty,
    aiReady,
    onstarter,
    onretry,
  }: {
    emptyVariant: "ready" | "off" | "unreachable";
    /// True once the capability read settled, so the empty state never
    /// flashes the wrong variant.
    showEmpty: boolean;
    aiReady: boolean;
    onstarter: (text: string) => void;
    onretry: () => void;
  } = $props();

  let scrollEl = $state<HTMLDivElement | null>(null);
  let atBottom = $state(true);

  function onScroll() {
    if (!scrollEl) return;
    atBottom = scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight < 80;
  }

  function scrollToBottom(smooth = true) {
    scrollEl?.scrollTo({ top: scrollEl.scrollHeight, behavior: smooth ? "smooth" : "auto" });
  }

  function scrollToId(id: number): void {
    scrollEl?.querySelector(`[data-mid="${id}"]`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  }

  // Jump to a bookmarked turn on request (the bookmarks affordance lives in the
  // composer foot; this owns the scroll region). One-shot: consume and reset.
  $effect(() => {
    const id = $jumpToMessage;
    if (id === null) return;
    scrollToId(id);
    jumpToMessage.set(null);
  });

  // Find in chat: the bar filters the current conversation and steps through the
  // matching turns, scrolling + highlighting the current one. Reuses the tested
  // matchingMessages + the data-mid anchors.
  let query = $state("");
  let matchIndex = $state(0);
  let findInput = $state<HTMLInputElement | null>(null);
  const matches = $derived(matchingMessages($messages, query));
  const currentId = $derived(matches[matchIndex]?.id ?? null);

  // A new query starts at the first hit.
  $effect(() => {
    void query;
    matchIndex = 0;
  });
  // Scroll to the current match whenever it changes while finding.
  $effect(() => {
    if ($findOpen && currentId !== null) scrollToId(currentId);
  });
  // Focus the input when the bar opens (from Ctrl+F or the foot icon).
  $effect(() => {
    if ($findOpen) requestAnimationFrame(() => findInput?.focus());
  });

  function stepMatch(delta: number): void {
    if (matches.length === 0) return;
    matchIndex = (matchIndex + delta + matches.length) % matches.length;
  }
  function closeFind(): void {
    findOpen.set(false);
    query = "";
    matchIndex = 0;
  }
  function onFindKeydown(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      e.preventDefault();
      stepMatch(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeFind();
    }
  }
  function onWindowKeydown(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      findOpen.set(true);
    }
  }

  // Follow the conversation: when a message is added or filled in while the
  // reader is at the bottom, keep them there. Scrolled-up readers are never
  // yanked away.
  let lastCount = 0;
  $effect(() => {
    const count = $messages.length;
    const pending = $messages[$messages.length - 1]?.pending;
    void pending;
    if (count !== lastCount) {
      lastCount = count;
      if (atBottom) tick().then(() => scrollToBottom());
    }
  });
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="thread-wrap">
  {#if $findOpen}
    <div class="find" role="search">
      <input
        bind:this={findInput}
        bind:value={query}
        class="find-input"
        placeholder={$t("h.thread.find")}
        aria-label={$t("h.thread.find")}
        onkeydown={onFindKeydown}
      />
      <span class="find-count">
        {matches.length ? `${matchIndex + 1} of ${matches.length}` : query ? "No matches" : ""}
      </span>
      <button class="find-btn" type="button" aria-label={$t("h.thread.prevMatch")} disabled={matches.length === 0} onclick={() => stepMatch(-1)}>
        <ChevronUp size={15} strokeWidth={2} />
      </button>
      <button class="find-btn" type="button" aria-label={$t("h.thread.nextMatch")} disabled={matches.length === 0} onclick={() => stepMatch(1)}>
        <ChevronDown size={15} strokeWidth={2} />
      </button>
      <button class="find-btn" type="button" aria-label={$t("h.thread.closeFind")} onclick={closeFind}>
        <X size={15} strokeWidth={2} />
      </button>
    </div>
  {/if}

  <div class="thread-scroll" bind:this={scrollEl} onscroll={onScroll}>
    {#if $messages.length === 0}
      {#if showEmpty}
        <EmptyState variant={emptyVariant} {onstarter} {onretry} />
      {/if}
    {:else}
      <div
        class="thread"
        role="log"
        aria-live="polite"
        aria-relevant="additions"
        aria-label={$t("h.thread.conversation")}
      >
        {#each $messages as message (message.id)}
          <ChatMessage {message} {aiReady} highlighted={message.id === currentId} />
        {/each}
      </div>
    {/if}
  </div>

  {#if !atBottom && $messages.length > 0}
    <button type="button" class="jump" aria-label={$t("h.thread.jumpLatest")} onclick={() => scrollToBottom()}>
      <ChevronDown size={16} strokeWidth={2} />
    </button>
  {/if}

  <!-- One shared right-click menu for every file-reference pill in the
       transcript (fixed-positioned, opened at the cursor via the store). -->
  <FileRefMenu />
</div>

<style>
  .thread-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .thread-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: var(--space-page, 1.5rem);
    padding-bottom: var(--space-section, 1.5rem);
  }
  .thread {
    display: flex;
    flex-direction: column;
    gap: var(--space-section, 1.5rem);
    max-width: var(--width-thread, 48rem);
    margin-inline: auto;
    padding-inline: var(--space-page, 1.5rem);
  }
  /* Floating jump-back control, centered on the column just above the
     composer zone. */
  .jump {
    position: absolute;
    left: 50%;
    bottom: 0.75rem;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-full);
    background: var(--color-bg-card);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    z-index: 5;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .jump:hover {
    background: color-mix(in srgb, var(--foreground) 8%, var(--color-bg-card));
    color: var(--foreground);
  }
  /* The find bar: on-demand, top-right of the reading column (not a persistent
     control). Appears only while finding. */
  .find {
    position: absolute;
    top: 0.75rem;
    right: 0.75rem;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.25rem 0.25rem 0.6rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input, 8px);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg, 0 8px 30px #00000066);
  }
  .find-input {
    width: 12rem;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.8125rem;
    outline: none;
  }
  .find-input::placeholder {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .find-count {
    min-width: 4rem;
    text-align: right;
    font-size: 0.75rem;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .find-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: var(--radius-button, 6px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  .find-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .find-btn:disabled {
    opacity: 0.4;
  }
</style>
