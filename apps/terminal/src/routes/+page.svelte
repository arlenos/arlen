<script lang="ts">
  /// The console surface: one continuous xterm.js for the active session
  /// (terminal.md re-architecture - engine-down, renderer-out). The Rust engine
  /// pumps raw PTY bytes; xterm.js owns the VT parsing, render, scrollback,
  /// input and focus, so there is no DOM-cell grid and no per-frame grid IPC.
  /// The block model survives as OSC 133 marker decorations over the grid
  /// (Terminal.svelte) plus the engine's block records (sidebar + history). The
  /// centered boxes exist only for the two failure cases (backend unreachable,
  /// session would not start).
  import { onMount } from "svelte";
  import {
    sessions,
    activeSessionId,
    sessionsLoaded,
    sessionsError,
    loadSessions,
    newSession,
    initSessionExitListener,
  } from "$lib/stores/sessions";
  import Terminal from "$lib/components/Terminal.svelte";
  import StreamEmpty from "$lib/components/StreamEmpty.svelte";
  import { t, dir } from "$lib/i18n/messages";

  onMount(() => {
    loadSessions();
    // Shell exit closes its session (and the window when it was the last).
    void initSessionExitListener();
  });
</script>

<!-- The console hosts one continuous xterm.js grid. xterm.js owns input, focus
     and the VT render, so the wrapper carries no tabindex/keystroke handler; it
     just suppresses the browser context menu so right-click word-select belongs
     to the terminal. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="console" dir={$dir} oncontextmenu={(e) => e.preventDefault()}>
  <div class="stream">
    {#if $sessionsLoaded && $sessionsError}
      <StreamEmpty kind="unreachable" onretry={() => loadSessions()} />
    {:else if $sessionsLoaded && $sessions.length === 0}
      <StreamEmpty kind="none" onretry={() => newSession()} />
    {:else if $activeSessionId}
      <!-- One xterm.js for the whole session: live grid, scrollback and any
           fullscreen TUI all flow through the same instance (xterm switches to
           the alternate buffer itself), so it is never remounted on an
           alt-screen toggle and never loses scrollback or the PTY connection.
           Block boundaries are OSC 133 decorations inside it; the DOM block
           stream is gone (it double-rendered against the grid). -->
      <Terminal sessionId={$activeSessionId} />
    {/if}
  </div>
</div>

<style>
  .console {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
    /* A terminal surface shows no browser focus ring; xterm.js draws the cursor
       as the focus indicator. */
    outline: none;
  }

  /* The xterm.js host fills this box; xterm owns its own scrollback + scrollbar,
     so the stream itself never scrolls. A small inset keeps the grid off the
     window edge. */
  .stream {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    padding: 4px 8px;
  }

</style>
