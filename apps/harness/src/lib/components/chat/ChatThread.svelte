<script lang="ts">
  /// The flat document message list: owns the scroll region, follows new
  /// messages while the reader is at the bottom, and offers a jump-back
  /// control after scrolling up. Hosts the empty state when there is
  /// nothing to show.
  import { tick } from "svelte";
  import { ChevronDown, Bookmark } from "@lucide/svelte";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import ChatMessage from "./ChatMessage.svelte";
  import EmptyState from "./EmptyState.svelte";
  import FileRefMenu from "./FileRefMenu.svelte";
  import { messages } from "$lib/stores/conversation";
  import { pinnedMessages } from "$lib/bookmark";

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

  // The bookmarked turns of this conversation, and jumping to one. The affordance
  // is the promised home for the per-message bookmark action.
  const pinned = $derived(pinnedMessages($messages));
  let bookmarksOpen = $state(false);

  function scrollToMessage(id: number) {
    scrollEl?.querySelector(`[data-mid="${id}"]`)?.scrollIntoView({ behavior: "smooth", block: "center" });
    bookmarksOpen = false;
  }
  function snippet(text: string): string {
    const t = text.trim().replace(/\s+/g, " ");
    return t.length > 64 ? `${t.slice(0, 64)}…` : t;
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

<div class="thread-wrap">
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
        aria-label="Conversation"
      >
        {#each $messages as message (message.id)}
          <ChatMessage {message} {aiReady} />
        {/each}
      </div>
    {/if}
  </div>

  {#if !atBottom && $messages.length > 0}
    <button type="button" class="jump" aria-label="Jump to latest" onclick={() => scrollToBottom()}>
      <ChevronDown size={16} strokeWidth={2} />
    </button>
  {/if}

  <!-- The bookmarks affordance: appears only when the conversation has bookmarked
       turns, and jumps to one. The home the per-message bookmark action was missing. -->
  {#if pinned.length > 0}
    <Popover.Root bind:open={bookmarksOpen}>
      <Popover.Trigger>
        {#snippet child({ props })}
          <button {...props} class="bookmarks-btn" title="Bookmarked messages">
            <Bookmark size={14} strokeWidth={2} />
            <span class="bm-count">{pinned.length}</span>
          </button>
        {/snippet}
      </Popover.Trigger>
      <Popover.Content align="end" class="bm-pop">
        <p class="bm-title">Bookmarks</p>
        <div class="bm-list">
          {#each pinned as m (m.id)}
            <button type="button" class="bm-item" onclick={() => scrollToMessage(m.id)}>
              <span class="bm-role">{m.role === "user" ? "You" : "Assistant"}</span>
              <span class="bm-text">{snippet(m.text)}</span>
            </button>
          {/each}
        </div>
      </Popover.Content>
    </Popover.Root>
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
  /* Floating bookmarks control, top-right of the reading column. Mirrors the
     jump control; carries the count so it reads as a quiet, contextual entry. */
  .bookmarks-btn {
    position: absolute;
    right: 0.75rem;
    top: 0.75rem;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    height: var(--height-control, 28px);
    padding: 0 0.6rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-full);
    background: var(--color-bg-card);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font-size: 0.75rem;
    z-index: 5;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .bookmarks-btn:hover {
    background: color-mix(in srgb, var(--foreground) 8%, var(--color-bg-card));
    color: var(--foreground);
  }
  .bm-count {
    font-variant-numeric: tabular-nums;
  }
  /* The popover content portals out of this subtree, so its inner styles are
     global (scoped by the .bm-pop wrapper class). */
  :global(.bm-pop) {
    width: 20rem;
    max-width: calc(100vw - 2rem);
    padding: 0.4rem;
  }
  :global(.bm-pop .bm-title) {
    margin: 0.15rem 0.4rem 0.35rem;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  :global(.bm-pop .bm-list) {
    display: flex;
    flex-direction: column;
    max-height: 20rem;
    overflow-y: auto;
  }
  :global(.bm-pop .bm-item) {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    width: 100%;
    padding: 0.4rem 0.45rem;
    border: none;
    border-radius: var(--radius-md, 6px);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  :global(.bm-pop .bm-item:hover) {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  :global(.bm-pop .bm-role) {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  :global(.bm-pop .bm-text) {
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
