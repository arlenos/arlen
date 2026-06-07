<script lang="ts">
  /// Conversation history rail (ai-app.md §2.0, chat archetype's contextual
  /// second column): "New chat" plus the list of conversations, click to switch.
  ///
  /// A8 inc 1: sessions are in-memory for the run (the store holds them); disk
  /// persistence so they survive a restart is the next sub-increment. The frame
  /// and wiring are already what persistence will fill.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Plus, MessageSquare } from "@lucide/svelte";
  import { sessions, activeSessionId, newSession, selectSession } from "$lib/stores/conversation";
</script>

<aside class="rail" aria-label="Conversations">
  <div class="rail-head">
    <Button variant="outline" size="sm" class="rail-new" onclick={() => newSession()}>
      <Plus size={14} strokeWidth={2} />
      New chat
    </Button>
  </div>
  <div class="rail-body">
    {#if $sessions.length === 0}
      <div class="rail-empty">
        <MessageSquare size={18} strokeWidth={1.5} />
        <span>No conversations yet.</span>
        <span class="rail-empty-sub">Ask something to start one.</span>
      </div>
    {:else}
      <ul class="rail-list">
        {#each $sessions as s (s.id)}
          <li>
            <button
              class="rail-item"
              class:active={s.id === $activeSessionId}
              onclick={() => selectSession(s.id)}
              title={s.title}
            >
              <MessageSquare size={13} strokeWidth={1.75} />
              <span class="rail-item-title">{s.title}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</aside>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    width: 15rem;
    flex-shrink: 0;
    min-height: 0;
    border-right: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--color-bg-card) 35%, transparent);
  }
  .rail-head {
    padding: 0.6rem;
    border-bottom: 1px solid var(--color-border);
  }
  /* The button stretches to the rail width so the action reads as the rail's
     primary affordance, like a chat client's New-chat row. */
  .rail-head :global(.rail-new) {
    width: 100%;
    justify-content: flex-start;
    gap: 0.4rem;
  }
  .rail-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.5rem;
  }
  .rail-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.3rem;
    padding: 1.5rem 0.75rem;
    text-align: center;
    font-size: 0.78rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rail-empty-sub {
    font-size: 0.72rem;
    color: color-mix(in srgb, var(--foreground) 38%, transparent);
  }
  .rail-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .rail-item {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    width: 100%;
    padding: 0.4rem 0.5rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 78%, transparent);
    font-size: 0.8rem;
    text-align: left;
    border-radius: var(--radius-chip);
    cursor: pointer;
  }
  .rail-item :global(svg) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rail-item:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .rail-item.active {
    background: color-mix(in srgb, var(--color-accent) 16%, transparent);
    color: var(--foreground);
  }
  .rail-item.active :global(svg) {
    color: var(--color-accent);
  }
  .rail-item-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* On a narrow window the rail yields so the chat keeps usable width. */
  @media (max-width: 52rem) {
    .rail {
      display: none;
    }
  }
</style>
