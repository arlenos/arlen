<script lang="ts">
  /// The block stream: the session's command history as quiet blocks,
  /// newest at the bottom like a terminal. The live prompt (the
  /// composer) rides along as the last row — at the top of an empty
  /// session, directly under the last block otherwise. Sticks to the
  /// bottom while the user is there; scrolling up parks the view and
  /// offers a jump pill instead of yanking it back down.
  import type { Snippet } from "svelte";
  import type { Block } from "$lib/contract";
  import StreamBlock from "./StreamBlock.svelte";

  let {
    blocks,
    children,
  }: {
    blocks: Block[];
    /// The live prompt row (the composer), rendered after the blocks
    /// inside the scroller.
    children?: Snippet;
  } = $props();

  let scroller = $state<HTMLDivElement | null>(null);
  let content = $state<HTMLDivElement | null>(null);
  let pinnedToBottom = $state(true);

  function onScroll() {
    if (!scroller) return;
    const gap =
      scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    pinnedToBottom = gap < 24;
  }

  function jumpToLatest() {
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
  }

  // Anything that grows the content (new blocks, the composer gaining
  // lines, an image finishing its load) keeps the view glued to the
  // bottom — but only when the user already was there.
  $effect(() => {
    if (!content || !scroller) return;
    const observer = new ResizeObserver(() => {
      if (pinnedToBottom && scroller) {
        scroller.scrollTop = scroller.scrollHeight;
      }
    });
    observer.observe(content);
    return () => observer.disconnect();
  });
</script>

<div class="block-stream" bind:this={scroller} onscroll={onScroll}>
  <div class="stream-content" bind:this={content}>
    {#each blocks as block (block.id)}
      <StreamBlock {block} />
    {/each}
    {@render children?.()}
  </div>
</div>

{#if !pinnedToBottom}
  <div class="jump-wrap">
    <button class="jump-btn" onclick={jumpToLatest}>Jump to latest</button>
  </div>
{/if}

<style>
  .block-stream {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .stream-content {
    display: flex;
    flex-direction: column;
  }

  .jump-wrap {
    position: relative;
    height: 0;
    display: flex;
    justify-content: center;
  }
  .jump-btn {
    position: absolute;
    bottom: 10px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: var(--height-control-compact, 24px);
    padding: 0 10px;
    border-radius: var(--radius-full);
    border: 1px solid var(--control-border);
    background: var(--background);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: var(--text-xs);
    font-weight: 500;
    box-shadow: var(--shadow-md);
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .jump-btn:hover {
    background: var(--control-bg-hover);
    color: var(--foreground);
  }
</style>
