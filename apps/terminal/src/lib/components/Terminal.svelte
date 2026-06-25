<script lang="ts">
  // The real terminal grid: an xterm.js instance fed the raw PTY byte stream
  // (terminal.md re-architecture, Tim: engine-down, renderer-out). The Rust
  // engine pumps raw bytes; xterm.js owns the VT parsing + render, so the grid
  // is a GPU/canvas surface themed by a palette - NOT the DOM-span-per-cell
  // render. The block frame, inline images and artifacts stay web-UI around
  // this; only the live grid is xterm.js.
  import { onMount, onDestroy } from "svelte";
  import { Terminal, type IMarker } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { CanvasAddon } from "@xterm/addon-canvas";
  import "@xterm/xterm/css/xterm.css";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { terminalDrainOutput, terminalInput, terminalResize } from "$lib/contract";
  // arlen-ui owns the look; we only wire it. The ITheme + font tokens make the
  // grid the Arlen palette (not bare black), and the block-chrome builders are
  // anchored to the OSC 133 marks below so the visible block frame is restored.
  import {
    arlenTerminalTheme,
    TERMINAL_FONT_FAMILY,
    TERMINAL_FONT_SIZE,
    TERMINAL_LINE_HEIGHT,
  } from "$lib/terminal-theme";
  import { renderBlockResult } from "$lib/block-chrome";
  import { classifyMark, parseExitCode } from "$lib/block-marks";
  import "$lib/block-chrome.css";

  let { sessionId }: { sessionId: string } = $props();

  let host: HTMLDivElement;
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let unlistenFrame: UnlistenFn | undefined;
  let resizeObserver: ResizeObserver | undefined;

  // Pull the bytes the engine buffered since the last drain and feed them to
  // xterm.js. Called on every `terminal://frame` pulse (the engine signals it
  // after each PTY read), so output repaints within a frame, not on a timer.
  async function drain(): Promise<void> {
    if (!term) return;
    const bytes = await terminalDrainOutput(sessionId);
    if (bytes.length > 0) term.write(new Uint8Array(bytes));
  }

  // Renderer choice (terminal.md §9 WebKitGTK caveat). xterm.js's WebGL text
  // rendering has a documented history of breaking on WebKitGTK - missing glyphs,
  // or only cell backgrounds - and that failure is SILENT: the addon loads
  // without throwing and never fires `contextlost`, so a try/catch + onContextLoss
  // guard cannot catch it (it only covers no-context and context-loss, not a
  // present-but-mis-rendering context). A silently broken grid is the worst
  // outcome for the top-priority terminal, and WebGL on real WebKitGTK hardware
  // cannot be pixel-verified headlessly (the Xvfb GL stack is not a real GPU). So
  // the Linux default is the canvas renderer - still the 5-45x win over the DOM
  // renderer, and reliable. Flip PREFER_WEBGL to true once a real-hardware shot
  // confirms WebGL glyphs render; the onContextLoss path then degrades to canvas.
  const PREFER_WEBGL = false;

  function loadRenderer(t: Terminal): void {
    if (PREFER_WEBGL) {
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => {
          webgl.dispose();
          try {
            t.loadAddon(new CanvasAddon());
          } catch {
            /* DOM renderer remains */
          }
        });
        t.loadAddon(webgl);
        return;
      } catch {
        /* WebGL2 context unavailable - fall through to canvas */
      }
    }
    try {
      t.loadAddon(new CanvasAddon());
    } catch {
      /* DOM renderer remains as the last resort */
    }
  }

  // Block boundaries over the continuous grid (terminal.md approach B, VS Code's
  // way): the shell's OSC 133 marks delimit commands. At rest the terminal is
  // pure (no rails, no rules) - the only persistent chrome is the result strip
  // (exit + duration) anchored to the right of each prompt row. The block
  // structure surfaces on interaction: arlen-ui's hover tint spans a block's rows
  // when the pointer is over it and reveals run-again (see the hover-wiring spec
  // in arlen-ui-reports.md - the pointer->block mapping is the open coder piece).
  // The grid stays one canvas; the chrome is an overlay hung off the marker rows.
  function registerBlockChrome(t: Terminal): void {
    // Per-block state, reset at each prompt-start.
    let promptMarker: IMarker | undefined;
    let execStartMs: number | undefined;

    function onPromptStart(): void {
      // A new block begins here; remember its prompt row so the result strip can
      // anchor to it when the command ends.
      const marker = t.registerMarker(0) ?? undefined;
      promptMarker = marker;
      execStartMs = undefined;
    }

    function onCommandEnd(data: string): void {
      // Anchor the result strip to the right of the prompt row.
      const exitCode = parseExitCode(data);
      const durationMs =
        execStartMs !== undefined ? Date.now() - execStartMs : null;
      if (promptMarker && !promptMarker.isDisposed) {
        // Full-width over the prompt row (NOT anchor:"right" - that positions
        // relative to the marker column over the prompt, not the viewport edge,
        // so the strip landed on top of the prompt). block-chrome.css right-aligns
        // the strip's content within this full-width box (arlen-ui's anchor spec).
        const result = t.registerDecoration({
          marker: promptMarker,
          x: 0,
          width: t.cols,
        });
        result?.onRender((el) => renderBlockResult(el, { exitCode, durationMs }));
      }
      promptMarker = undefined;
      execStartMs = undefined;
    }

    // Both OSC 133 (FinalTerm) and OSC 633 (VS Code) carry the same A/C/D block
    // marks. The Arlen shell integration (arlen-shell-integration.zsh) emits
    // 633;A for prompt-start and 133;C / 133;D for exec/end (and 633;E for the
    // command line, which we ignore - the engine decodes + nonce-validates that
    // for the trusted block record), so route either family by the leading
    // letter. Run-again is omitted here on purpose: replaying a command must use
    // the engine's validated record, never a 633;E we decode in the webview.
    const dispatch = (data: string): boolean => {
      const mark = classifyMark(data);
      if (mark === "prompt-start") onPromptStart();
      else if (mark === "exec-start") execStartMs = Date.now();
      else if (mark === "command-end") onCommandEnd(data);
      // Return false so xterm's other handlers still run; the engine parses its
      // own raw copy of the PTY stream, so this never starves its block parser.
      return false;
    };
    t.parser.registerOscHandler(133, dispatch);
    t.parser.registerOscHandler(633, dispatch);
  }

  onMount(() => {
    const t = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      // Focus-aware: a hollow cursor when the grid is not focused.
      cursorInactiveStyle: "outline",
      rightClickSelectsWord: true,
      allowProposedApi: true,
      // arlen-ui's palette + mono (the in-app grid is the Arlen theme, not bare
      // black). xterm paints the grid on a canvas, so the colours must reach it
      // as the options object, never via CSS.
      theme: arlenTerminalTheme,
      fontFamily: TERMINAL_FONT_FAMILY,
      fontSize: TERMINAL_FONT_SIZE,
      lineHeight: TERMINAL_LINE_HEIGHT,
    });
    term = t;
    fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host);
    loadRenderer(t);
    registerBlockChrome(t);

    // The grid IS the keystroke target now (not a textbox): xterm.js emits the
    // UTF-8 input string, the engine writes it to the PTY master.
    t.onData((d) => void terminalInput(sessionId, d));
    // When xterm.js recomputes the geometry (on fit), resize the PTY to match so
    // the shell + TUIs reflow. Registered BEFORE the first fit() below: the
    // initial fit emits a resize, and if its handler is not yet attached that
    // event is lost and the PTY stays at its 80x24 spawn size - an alt-screen
    // TUI then draws only 24 rows into a taller grid (the under-fill bug).
    t.onResize(({ cols, rows }) => void terminalResize(sessionId, cols, rows));

    void listen<string>("terminal://frame", (e) => {
      if (e.payload === sessionId) void drain();
    }).then((un) => (unlistenFrame = un));

    // xterm.js owns input + focus now (its textarea is the keystroke target), so
    // focus it on mount: the cursor draws solid (not the inactive outline) and
    // keystrokes land without a click. xterm re-focuses on click too.
    t.focus();

    // Wait for the bundled mono (@fontsource/cascadia-code, imported in app.css)
    // to load before the first fit: xterm MEASURES the font to compute the cell
    // size, so a fit on the fallback face sizes the grid wrong (and the PTY with
    // it). Once the face is ready, size the grid to the host - the onResize above
    // syncs the PTY - start observing container resizes, and drain whatever the
    // engine already buffered before this view attached.
    void document.fonts.ready.then(() => {
      if (!term || !fit) return;
      fit.fit();
      resizeObserver = new ResizeObserver(() => fit?.fit());
      resizeObserver.observe(host);
      void drain();
    });
  });

  onDestroy(() => {
    unlistenFrame?.();
    resizeObserver?.disconnect();
    term?.dispose();
  });
</script>

<div class="terminal-host" bind:this={host}></div>
