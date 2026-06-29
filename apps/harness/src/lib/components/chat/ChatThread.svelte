<script lang="ts">
  /// The flat document message list: owns the scroll region, follows new
  /// messages while the reader is at the bottom, and offers a jump-back
  /// control after scrolling up. Hosts the empty state when there is
  /// nothing to show.
  import { tick } from "svelte";
  import { ChevronDown } from "@lucide/svelte";
  import ChatMessage from "./ChatMessage.svelte";
  import EmptyState from "./EmptyState.svelte";
  import FileRefMenu from "./FileRefMenu.svelte";
  import { messages } from "$lib/stores/conversation";

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
      <div class="thread">
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
</style>
