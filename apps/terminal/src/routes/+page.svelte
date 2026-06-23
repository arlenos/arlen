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
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    terminalBlocks,
    terminalGrid,
    terminalInput,
    terminalResize,
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
  import { liveRegionCells, isAltScreenActive, liveCursor } from "$lib/live-region";
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
  // Where to paint the block cursor within the live slice (null when hidden).
  const liveCur = $derived(liveCursor($liveGrid, liveCells));

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

  // Resize the PTY to match the rendered grid: a hidden probe (10 chars of the
  // console mono font over one line) gives the true cell width/height, so cols/
  // rows are computed from the real font rather than a hardcoded guess. A
  // ResizeObserver on the console fires terminal_resize (debounced) on any size
  // change, so the shell + any running TUI reflow via SIGWINCH (today resize did
  // nothing - the PTY kept its initial 80x24).
  let probeEl = $state<HTMLSpanElement | null>(null);
  let resizeTimer: ReturnType<typeof setTimeout> | undefined;

  function sendResize() {
    const id = $activeSessionId;
    if (!id || !consoleEl || !probeEl) return;
    const probe = probeEl.getBoundingClientRect();
    const cellW = probe.width / 10;
    const cellH = probe.height;
    // The probe may not be laid out yet at the first observer fire; retry next
    // frame rather than silently bail, so the INITIAL resize always lands. Until
    // it does the PTY stays at its 80x24 spawn size, which constrains every
    // command's output to 80 columns and makes a fullscreen TUI (btop) fill only
    // part of a larger window.
    if (cellW <= 0 || cellH <= 0) {
      requestAnimationFrame(sendResize);
      return;
    }
    const cols = Math.max(1, Math.floor(consoleEl.clientWidth / cellW));
    const rows = Math.max(1, Math.floor(consoleEl.clientHeight / cellH));
    terminalResize(id, cols, rows).catch(() => {});
  }

  $effect(() => {
    if (!consoleEl || !tauriAvailable) return;
    const observer = new ResizeObserver(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(sendResize, 80);
    });
    observer.observe(consoleEl);
    return () => {
      observer.disconnect();
      clearTimeout(resizeTimer);
    };
  });

  onMount(() => {
    loadSessions();
  });

  $effect(() => {
    const id = $activeSessionId;
    loadBlocks(id);
  });

  // Render on change, not on a fixed timer. The engine pushes a `terminal://frame`
  // ping whenever this session's screen changes (an echoed keystroke, new output),
  // so the live grid repaints within a frame instead of waiting up to a poll
  // interval — the latency that made it feel like a textbox. Fetches are coalesced
  // (one in flight, re-run if a frame landed during it) so a flood of pings can't
  // pile up. A slow safety poll guarantees the screen can never stay stale if a
  // ping is missed or the listener races session start; an idle prompt otherwise
  // does no work. Torn down on session change and unmount.
  $effect(() => {
    const id = $activeSessionId;
    if (!id || !tauriAvailable) {
      liveGrid.set(null);
      return;
    }
    let alive = true;
    let fetching = false;
    let dirtyWhileFetching = false;
    // Track the running flag so a command finishing (running true -> false) pulls
    // the freshly-closed block. The live region only ever shows the current prompt
    // and a running command's output; once a command ends its output moves into an
    // OSC133 block, which the block stream loads on session change only. Without
    // this a fast command's output would flash in the live region and vanish (the
    // block never re-fetched). Starts false so the first paint of an idle prompt
    // does not spuriously reload.
    let prevRunning = false;
    const fetchGrid = async () => {
      if (!alive) return;
      if (fetching) {
        dirtyWhileFetching = true;
        return;
      }
      fetching = true;
      try {
        const grid = await terminalGrid(id);
        if (alive) {
          liveGrid.set(grid);
          if (prevRunning && !grid.running) void loadBlocks(id);
          prevRunning = grid.running;
        }
      } catch {
        // Keep the last good screen on a transient read failure.
      } finally {
        fetching = false;
        if (alive && dirtyWhileFetching) {
          dirtyWhileFetching = false;
          void fetchGrid();
        }
      }
    };
    // Initial paint, then repaint on each frame ping for this session.
    void fetchGrid();
    let unlisten: UnlistenFn | null = null;
    let disposed = false;
    void listen<string>("terminal://frame", (e) => {
      if (e.payload === id) void fetchGrid();
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    const safety = setInterval(fetchGrid, 1000);
    return () => {
      alive = false;
      disposed = true;
      unlisten?.();
      clearInterval(safety);
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
  oncontextmenu={(e) => e.preventDefault()}
>
  <!-- Hidden cell-size probe in the exact console cell font, for the resize
       computation; not shown to AT or the user. -->
  <span bind:this={probeEl} class="cell-probe" aria-hidden="true">0000000000</span>
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
        <GridRegion cells={liveCells} cursorRow={liveCur?.row ?? null} cursorCol={liveCur?.col ?? null} />
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
            <GridRegion cells={liveCells} cursorRow={liveCur?.row ?? null} cursorCol={liveCur?.col ?? null} />
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
    /* The console is the focusable keystroke surface (tabindex + role), and it
       is focused for the whole session, so the browser focus ring draws a
       (reddish) line along its top edge - a web idiom on a terminal. Suppress
       it: the shell's cursor + prompt are the focus indicator (same as the
       alt-screen surface below). */
    outline: none;
  }

  /* Off-screen probe in the exact console cell font, measured to derive the
     cell width/height for the PTY resize computation. Must track the grid's
     font-size (--console-font-size) one-to-one or the measured cell size
     diverges from the painted one and the columns stop aligning. */
  .cell-probe {
    position: absolute;
    top: -9999px;
    left: -9999px;
    visibility: hidden;
    white-space: pre;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--console-font-size, 0.8125rem);
    line-height: 1.5;
    pointer-events: none;
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
