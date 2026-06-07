<script lang="ts">
  /// Conversation history rail (ai-app.md §2.0, chat archetype's contextual
  /// second column). Holds "New chat" and the session list.
  ///
  /// Session persistence is not built yet (A8), so the list is an honest
  /// empty-state rather than a fake column. When the session store lands it
  /// drives this list (search, switch, resume); the frame is already here.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Plus, MessageSquare } from "@lucide/svelte";
  import { reset } from "$lib/stores/conversation";

  // Until persistence exists, "New chat" simply clears the current thread.
  function newChat() {
    reset();
  }
</script>

<aside class="rail" aria-label="Conversations">
  <div class="rail-head">
    <Button variant="outline" size="sm" class="rail-new" onclick={newChat}>
      <Plus size={14} strokeWidth={2} />
      New chat
    </Button>
  </div>
  <div class="rail-body">
    <div class="rail-empty">
      <MessageSquare size={18} strokeWidth={1.5} />
      <span>No saved conversations yet.</span>
      <span class="rail-empty-sub">History arrives with session persistence.</span>
    </div>
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
  /* On a narrow window the rail yields so the chat keeps usable width. */
  @media (max-width: 52rem) {
    .rail {
      display: none;
    }
  }
</style>
