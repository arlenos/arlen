<script lang="ts">
  /// The console surface: the active session's block stream with the
  /// composer pinned below. Block data lands in a writable store
  /// (IPC-callback rule); the stream re-loads when the active session
  /// changes.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { terminalBlocks, type Block } from "$lib/contract";
  import {
    activeSessionId,
    sessionsLoaded,
    loadSessions,
    newSession,
  } from "$lib/stores/sessions";
  import BlockStream from "$lib/components/BlockStream.svelte";

  const blocks = writable<Block[]>([]);
  const blocksLoaded = writable(false);

  async function loadBlocks(sessionId: string | null) {
    if (!sessionId) {
      blocks.set([]);
      blocksLoaded.set(true);
      return;
    }
    try {
      blocks.set(await terminalBlocks(sessionId));
    } catch {
      blocks.set([]);
    }
    blocksLoaded.set(true);
  }

  onMount(() => {
    loadSessions();
  });

  $effect(() => {
    const id = $activeSessionId;
    loadBlocks(id);
  });
</script>

<div class="console">
  <div class="stream">
    {#if $sessionsLoaded && $activeSessionId === null}
      <div class="stream-empty">
        <span class="stream-empty-title">No shell is open</span>
        <span class="stream-empty-hint">
          Start one with the plus button in the sidebar or Ctrl+T.
        </span>
        <button class="stream-empty-btn" onclick={() => newSession()}>
          New session
        </button>
      </div>
    {:else if $blocksLoaded && $blocks.length === 0 && $activeSessionId !== null}
      <div class="stream-empty">
        <span class="stream-empty-title">Fresh shell</span>
        <span class="stream-empty-hint">
          Commands you run appear here as blocks.
        </span>
      </div>
    {:else}
      <BlockStream blocks={$blocks} />
    {/if}
  </div>
</div>

<style>
  .console {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }

  .stream {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .stream-empty {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
    padding: 2rem;
  }
  .stream-empty-title {
    font-size: 0.9375rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .stream-empty-hint {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .stream-empty-btn {
    margin-top: 8px;
    height: var(--height-control, 28px);
    padding: 0 12px;
    border-radius: var(--radius-input);
    border: 1px solid var(--control-border);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.8125rem;
    font-weight: 500;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .stream-empty-btn:hover {
    background: var(--control-bg-hover);
  }
</style>
