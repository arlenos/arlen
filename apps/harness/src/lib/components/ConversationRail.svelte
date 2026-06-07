<script lang="ts">
  /// Conversation history rail (ai-app.md §2.0, chat archetype's contextual
  /// second column): "New chat", a title search, and the list of conversations,
  /// click to switch. Sessions persist to disk (A8), so the list survives a
  /// restart.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Plus, MessageSquare, Search, X } from "@lucide/svelte";
  import {
    sessions,
    activeSessionId,
    newSession,
    selectSession,
    deleteSession,
    renameSession,
  } from "$lib/stores/conversation";

  let query = $state("");
  // The conversation being renamed inline, and the draft title. `null` when no
  // rename is in progress. Double-clicking a title opens the editor on that row.
  let editingId = $state<string | null>(null);
  let draft = $state("");

  function beginRename(id: string, current: string): void {
    editingId = id;
    draft = current;
  }
  function commitRename(): void {
    if (editingId !== null) renameSession(editingId, draft);
    editingId = null;
  }
  function cancelRename(): void {
    editingId = null;
  }
  // Sessions whose title matches the search, case-insensitive. Empty query
  // matches everything.
  const filtered = $derived(
    $sessions.filter((s) =>
      s.title.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  );
</script>

<aside class="rail" aria-label="Conversations">
  <div class="rail-head">
    <Button variant="outline" size="sm" class="rail-new" onclick={() => newSession()}>
      <Plus size={14} strokeWidth={2} />
      New chat
    </Button>
  </div>
  {#if $sessions.length > 0}
    <div class="rail-search">
      <Search size={13} strokeWidth={2} />
      <input
        type="text"
        bind:value={query}
        placeholder="Search conversations"
        aria-label="Search conversations"
      />
    </div>
  {/if}
  <div class="rail-body">
    {#if $sessions.length === 0}
      <div class="rail-empty">
        <MessageSquare size={18} strokeWidth={1.5} />
        <span>No conversations yet.</span>
        <span class="rail-empty-sub">Ask something to start one.</span>
      </div>
    {:else if filtered.length === 0}
      <div class="rail-empty">
        <span>No conversations match.</span>
      </div>
    {:else}
      <ul class="rail-list">
        {#each filtered as s (s.id)}
          <li class="rail-row" class:active={s.id === $activeSessionId}>
            {#if editingId === s.id}
              <input
                class="rail-edit"
                bind:value={draft}
                aria-label="Rename conversation"
                onblur={commitRename}
                onkeydown={(e) => {
                  if (e.key === "Enter") commitRename();
                  else if (e.key === "Escape") cancelRename();
                }}
                {@attach (node) => {
                  node.focus();
                  node.select();
                }}
              />
            {:else}
              <button
                class="rail-item"
                onclick={() => selectSession(s.id)}
                ondblclick={() => beginRename(s.id, s.title)}
                title={s.title}
              >
                <MessageSquare size={13} strokeWidth={1.75} />
                <span class="rail-item-title">{s.title}</span>
              </button>
              <button
                class="rail-del"
                aria-label="Delete conversation"
                title="Delete conversation"
                onclick={(e) => {
                  e.stopPropagation();
                  deleteSession(s.id);
                }}
              >
                <X size={13} strokeWidth={2} />
              </button>
            {/if}
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
  .rail-search {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.45rem 0.6rem;
    border-bottom: 1px solid var(--color-border);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rail-search :global(svg) {
    flex-shrink: 0;
  }
  .rail-search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.8rem;
    outline: none;
  }
  .rail-search input::placeholder {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
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
  /* Row holds the select button and the hover-revealed delete button as
     siblings; the row carries the hover/active background so both sit on it. */
  .rail-row {
    display: flex;
    align-items: center;
    border-radius: var(--radius-chip);
  }
  .rail-row:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .rail-row.active {
    background: color-mix(in srgb, var(--color-accent) 16%, transparent);
  }
  .rail-item {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    flex: 1;
    min-width: 0;
    padding: 0.4rem 0.5rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 78%, transparent);
    font-size: 0.8rem;
    text-align: left;
    cursor: pointer;
  }
  .rail-item :global(svg) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rail-row.active .rail-item {
    color: var(--foreground);
  }
  .rail-row.active .rail-item :global(svg) {
    color: var(--color-accent);
  }
  /* Hidden until the row is hovered or its conversation is active, so the rail
     stays calm; always shown on keyboard focus for accessibility. */
  .rail-del {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    padding: 0.3rem;
    margin-right: 0.25rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    border-radius: var(--radius-chip);
    cursor: pointer;
    opacity: 0;
  }
  .rail-row:hover .rail-del,
  .rail-row.active .rail-del,
  .rail-del:focus-visible {
    opacity: 1;
  }
  .rail-del:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
  }
  .rail-item-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Inline rename editor: fills the row like the title it replaces, so the row
     does not jump when editing starts. */
  .rail-edit {
    flex: 1;
    min-width: 0;
    margin: 0.15rem 0.35rem;
    padding: 0.25rem 0.35rem;
    border: 1px solid color-mix(in srgb, var(--color-accent) 50%, transparent);
    border-radius: var(--radius-chip);
    background: var(--color-bg-card);
    color: var(--foreground);
    font-size: 0.8rem;
    outline: none;
  }
  /* On a narrow window the rail yields so the chat keeps usable width. */
  @media (max-width: 52rem) {
    .rail {
      display: none;
    }
  }
</style>
