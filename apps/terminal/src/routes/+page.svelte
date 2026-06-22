<script lang="ts">
  /// The console surface: the active session's block stream with the
  /// composer pinned below. Block data lands in a writable store
  /// (IPC-callback rule); the stream re-loads when the active session
  /// changes. A fresh session shows a silent stream and a focused
  /// composer — typing IS the empty state. The centered boxes exist
  /// only for the two failure cases (backend unreachable, session
  /// would not start).
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import {
    terminalBlocks,
    terminalGrid,
    type Block,
    type GridSnapshot,
  } from "$lib/contract";
  import {
    sessions,
    activeSessionId,
    sessionsLoaded,
    sessionsError,
    loadSessions,
    newSession,
  } from "$lib/stores/sessions";
  import { GridRegion } from "@arlen/ui-kit/components/console";
  import BlockStream from "$lib/components/BlockStream.svelte";
  import Composer from "$lib/components/Composer.svelte";

  const blocks = writable<Block[]>([]);
  // The live screen, polled from the engine's VT model (terminal.md
  // Option B). IPC continuations land in a store, never `$state`.
  const liveGrid = writable<GridSnapshot | null>(null);

  const tauriAvailable =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  async function loadBlocks(sessionId: string | null) {
    if (!sessionId) {
      blocks.set([]);
      return;
    }
    try {
      blocks.set(await terminalBlocks(sessionId));
    } catch {
      blocks.set([]);
    }
  }

  // The visible screen rows, trailing blank rows trimmed (but never below
  // the cursor row) so the live region is the height of the real output,
  // not the full 24-row grid. (A fullscreen/alt-screen app fills the grid;
  // honouring that without trimming is the follow-on once the snapshot
  // carries the alt-screen flag.)
  const liveCells = $derived.by(() => {
    const g = $liveGrid;
    if (!g) return [];
    let last = -1;
    for (let i = 0; i < g.cells.length; i++) {
      if (g.cells[i].some((cell) => cell.text.trim() !== "")) last = i;
    }
    last = Math.max(last, g.cursor_row);
    return g.cells.slice(0, last + 1);
  });

  onMount(() => {
    loadSessions();
  });

  $effect(() => {
    const id = $activeSessionId;
    loadBlocks(id);
  });

  // Poll the live screen while a session is active. The PTY streams
  // output asynchronously, so the screen is read on a timer (not only
  // on send); the interval is torn down on session change and unmount.
  $effect(() => {
    const id = $activeSessionId;
    if (!id || !tauriAvailable) {
      liveGrid.set(null);
      return;
    }
    let alive = true;
    const tick = async () => {
      try {
        const grid = await terminalGrid(id);
        if (alive) liveGrid.set(grid);
      } catch {
        // Keep the last good screen on a transient read failure.
      }
    };
    void tick();
    const timer = setInterval(tick, 120);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  const activeSession = $derived(
    $sessions.find((s) => s.id === $activeSessionId) ?? null,
  );

  // The live prompt inherits the last block's git for its cwd — the
  // truth as of the last command; live git is the engine's job.
  const promptGit = $derived.by(() => {
    const cwd = activeSession?.cwd;
    if (!cwd) return null;
    for (let i = $blocks.length - 1; i >= 0; i--) {
      const b = $blocks[i];
      if (b.cwd === cwd && b.git) return b.git;
    }
    return null;
  });
</script>

<div class="console">
  <div class="stream">
    {#if $sessionsLoaded && $sessionsError}
      <div class="stream-empty">
        <span class="stream-empty-title">Can't reach the shell backend</span>
        <span class="stream-empty-hint">The terminal engine did not answer.</span>
        <button class="stream-empty-btn" onclick={() => loadSessions()}>
          Try again
        </button>
      </div>
    {:else if $sessionsLoaded && $sessions.length === 0}
      <div class="stream-empty">
        <span class="stream-empty-title">Couldn't start a session</span>
        <span class="stream-empty-hint">The shell did not come up.</span>
        <button class="stream-empty-btn" onclick={() => newSession()}>
          New session
        </button>
      </div>
    {:else}
      <BlockStream blocks={$blocks}>
        {#if liveCells.length > 0}
          <!-- The live terminal screen (Option B): command output shows
               here even without the OSC-mark shell integration, which is
               what makes the empty-block-body case still display output. -->
          <div class="live-screen">
            <GridRegion cells={liveCells} />
          </div>
        {/if}
        <Composer
          session={activeSession}
          git={promptGit}
          onsent={() => loadBlocks($activeSessionId)}
        />
      </BlockStream>
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

  /* The live screen sits in the stream flow, above the composer. */
  .live-screen {
    padding: 4px 12px;
  }

  .stream-empty {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    text-align: center;
    padding: 2rem;
  }
  /* Chrome voice; the hierarchy is weight + dim, not size. */
  .stream-empty-title {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .stream-empty-hint {
    font-size: 0.75rem;
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
    font-size: 0.75rem;
    font-weight: 500;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .stream-empty-btn:hover {
    background: var(--control-bg-hover);
  }
</style>
