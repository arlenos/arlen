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
    terminalInput,
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
  import { liveRegionCells, isAltScreenActive } from "$lib/live-region";
  import { keyToBytes } from "$lib/keymap";
  import BlockStream from "$lib/components/BlockStream.svelte";

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

  // The live region rendered below the blocks: nothing at an idle prompt (the
  // composer is the prompt, finished output is in the blocks — no double
  // prompt), only a running command's output (sliced past the prompt + echo),
  // or the whole grid for a fullscreen alternate-screen app. The rule is the
  // pure, unit-tested `liveRegionCells` (live-region.ts).
  const liveCells = $derived(liveRegionCells($liveGrid));

  // A fullscreen / TUI app (btop, vim, less) holds the alternate screen, so the
  // block UI is turned off entirely and the grid fills the whole content area
  // (Tim's spec). Flips back to block-mode when the app exits the alt screen.
  const isAltScreen = $derived(isAltScreenActive($liveGrid, liveCells));

  // The whole console is the interactive surface (PR-2 raw-PTY input, no composer
  // textbox): every keystroke goes raw to the PTY, so the user's real shell runs
  // its line editor (zsh `zle`) and renders the prompt + the line being typed
  // itself (p10k, syntax-highlighting, autosuggestions), and a fullscreen TUI
  // gets its keys too. The handler lives on the `.console` wrapper so it covers
  // block-mode and alt-screen alike. `keyToBytes` returns null for copy/paste +
  // WM combos, which fall through to the browser/app.
  let consoleEl = $state<HTMLDivElement | null>(null);

  function onGridKey(e: KeyboardEvent) {
    const id = $activeSessionId;
    if (!id) return;
    const bytes = keyToBytes(e);
    if (bytes === null) return;
    e.preventDefault();
    terminalInput(id, bytes).catch(() => {});
  }

  // Focus the console when a session is active so keystrokes land without a
  // manual click. Not focused in the empty/error states (no active session), so
  // their buttons stay usable.
  $effect(() => {
    if ($activeSessionId && tauriAvailable) consoleEl?.focus();
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

</script>

<!-- The console is the interactive terminal surface: it takes keystrokes raw to
     the PTY (role=application passes keys through to the shell, the tabindex
     makes it focusable). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
<div
  class="console"
  bind:this={consoleEl}
  tabindex="0"
  role="application"
  onkeydown={onGridKey}
>
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
    {:else if isAltScreen}
      <!-- A fullscreen / TUI app holds the alternate screen: the block UI is
           turned off entirely and the app gets the whole content area as one VT
           grid (the sidebar lives outside .console, so it stays). Keystrokes
           reach it via the console-level handler. Switches back to block-mode
           when alt_screen flips false on exit. -->
      <div class="alt-fullscreen">
        <GridRegion cells={liveCells} />
      </div>
    {:else}
      <BlockStream blocks={$blocks}>
        {#if liveCells.length > 0}
          <!-- The live terminal screen IS the interactive surface: the real
               shell's prompt + the line being typed (zle, syntax-highlighting)
               at an idle prompt, and a running command's output. Keystrokes go
               raw to the PTY via the console-level handler - there is no composer
               textbox. Finished commands above stay in their blocks (the live
               region starts at the current prompt, so no double render). -->
          <div class="live-screen">
            <GridRegion cells={liveCells} />
          </div>
        {/if}
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

  /* Alt-screen / fullscreen TUI: the grid takes the whole content area, no
     block chrome, no composer (the sidebar stays, outside .console). */
  .alt-fullscreen {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    padding: 4px 8px;
    /* The grid is the keystroke target here; a terminal surface shows no
       browser focus ring. */
    outline: none;
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
