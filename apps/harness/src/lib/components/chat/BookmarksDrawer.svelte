<script lang="ts">
  /// The bookmarks drawer: the home for the per-message bookmark action, in the same
  /// right-side drawer form as the transparency drawer (summoned from the composer
  /// foot, mounted once in the layout). Lists the bookmarked turns of the active
  /// conversation; clicking one jumps the transcript to it.
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";
  import { X } from "@lucide/svelte";
  import { messages } from "$lib/stores/conversation";
  import { pinnedMessages } from "$lib/bookmark";
  import { bookmarksOpen, jumpToMessage } from "$lib/stores/chatNav";

  const pinned = $derived(pinnedMessages($messages));

  function close(): void {
    bookmarksOpen.set(false);
  }
  function jump(id: number): void {
    jumpToMessage.set(id);
    close();
  }
  function snippet(text: string): string {
    const t = text.trim().replace(/\s+/g, " ");
    return t.length > 90 ? `${t.slice(0, 90)}…` : t;
  }

  // Escape closes the drawer, like any overlay.
  onMount(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && $bookmarksOpen) {
        e.preventDefault();
        close();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });
</script>

{#if $bookmarksOpen}
  <div class="scrim" onclick={close} role="presentation"></div>
  <aside class="drawer" aria-label={$t("h.bookmarks.aria")}>
    <header class="head">
      <span class="head-title">{$t("h.bookmarks.title")}</span>
      <button class="x" aria-label={$t("h.bookmarks.close")} onclick={close}>
        <X size={15} strokeWidth={2} />
      </button>
    </header>

    <div class="body">
      {#if pinned.length === 0}
        <p class="empty">{$t("h.bookmarks.empty")}</p>
      {:else}
        {#each pinned as m (m.id)}
          <button type="button" class="row" onclick={() => jump(m.id)}>
            <span class="role">{m.role === "user" ? "You" : "Assistant"}</span>
            <span class="text">{snippet(m.text)}</span>
          </button>
        {/each}
      {/if}
    </div>
  </aside>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: color-mix(in srgb, #000 45%, transparent);
  }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 21;
    width: 24rem;
    max-width: 92vw;
    display: flex;
    flex-direction: column;
    background: var(--color-bg-app, #0f0f0f);
    border-left: 1px solid var(--color-border);
    box-shadow: -12px 0 40px rgba(0, 0, 0, 0.4);
    font-size: 0.875rem;
    color: var(--foreground, #fafafa);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 44px;
    padding: 0 0.5rem 0 1rem;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .head-title {
    font-size: 0.8125rem;
    font-weight: 500;
  }
  .x {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    border-radius: var(--radius-button, 6px);
    cursor: pointer;
  }
  .x:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.5rem;
  }
  .empty {
    padding: 0.75rem 0.5rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    width: 100%;
    padding: 0.55rem 0.6rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .row:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .role {
    font-size: 0.6875rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .text {
    font-size: 0.8125rem;
    line-height: 1.4;
    color: var(--foreground);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
