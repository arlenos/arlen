<script lang="ts">
  // The real terminal grid: an xterm.js instance fed the raw PTY byte stream
  // (terminal.md re-architecture, Tim: engine-down, renderer-out). The Rust
  // engine pumps raw bytes; xterm.js owns the VT parsing + render, so the grid
  // is a GPU/canvas surface themed by a palette - NOT the DOM-span-per-cell
  // render. The block frame, inline images and artifacts stay web-UI around
  // this; only the live grid is xterm.js.
  import { onMount, onDestroy } from "svelte";
  import { Terminal, type IMarker, type IDecoration } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { CanvasAddon } from "@xterm/addon-canvas";
  import "@xterm/xterm/css/xterm.css";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    terminalLastCommand,
    terminalDrainOutput,
    terminalInput,
    terminalResize,
    terminalSaveOutput,
  } from "$lib/contract";
  // arlen-ui owns the look; we only wire it. The ITheme + font tokens make the
  // grid the Arlen palette (not bare black), and the block-chrome builders are
  // anchored to the OSC 133 marks below so the visible block frame is restored.
  import {
    arlenTerminalTheme,
    TERMINAL_FONT_FAMILY,
    TERMINAL_FONT_SIZE,
    TERMINAL_LINE_HEIGHT,
  } from "$lib/terminal-theme";
  import { applyBlockHover, renderBlockResult } from "$lib/block-chrome";
  import { classifyMark, parseExitCode } from "$lib/block-marks";
  import "$lib/block-chrome.css";
  import BlockContextMenu from "./BlockContextMenu.svelte";

  let { sessionId }: { sessionId: string } = $props();

  // One finished block, tracked for the hover tint AND the right-click menu. Each
  // keeps its prompt marker, the output-start + end buffer lines, whether it
  // failed, its result-strip element, and the validated command. A marker
  // auto-disposes when its line is trimmed from scrollback.
  interface BlockEntry {
    promptMarker: IMarker;
    /// First output buffer line (the 133;C exec-start row); `undefined` for a
    /// command-less prompt block.
    outputStartLine?: number;
    endLine: number;
    isError: boolean;
    resultEl?: HTMLElement;
    /// The command this block ran (engine's validated record); replayed as a PTY
    /// write by run-again / edit-and-rerun.
    command?: string;
  }

  // The right-click menu's target: the block currently under the pointer (the one
  // the hover tint marks). A plain let, not $state - the menu's action closures
  // read it synchronously at click time, so no template reactivity is needed (and
  // the pointermove callback that sets it would not reliably drive $state anyway).
  let menuBlock: BlockEntry | undefined;

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

  // Renderer choice - decided by the block-chrome alignment blocker (#967), not
  // just perf. The three xterm renderers trade speed vs correctness HERE:
  //  - DOM (the DEFAULT, no addon): the block-chrome decorations land pixel-exact
  //    on every row - arlen-ui measured the decoration `offsetTop` == its
  //    `marker.line` text row on every line. The block frame MUST sit on its rows
  //    and the block model is the terminal's whole point, so this is the default.
  //  - Canvas / WebGL: faster, but they MIS-SCALE the grid at a fractional
  //    devicePixelRatio (xterm #967, Tim's display is 1.5x HiDPI): the canvas
  //    backs at 1.5x yet draws 1.0x cells, so the text rows compress (~11px pitch)
  //    and drift UP from the CSS-positioned decorations (17px pitch), the gap
  //    growing down the screen - exactly Tim's "passt gar nicht". WebGL also
  //    risks silent WebKitGTK text breakage (loads without throwing, renders
  //    wrong). Both are kept behind flags so the perf-vs-alignment trade can be
  //    re-evaluated on real hardware once the canvas DPR path (#967) is fixed +
  //    metal-pixel-verified; until then correctness wins and DOM is the default.
  const PREFER_WEBGL = false;
  const PREFER_CANVAS = false;

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
        /* WebGL2 context unavailable - fall through */
      }
    }
    if (PREFER_CANVAS) {
      try {
        t.loadAddon(new CanvasAddon());
      } catch {
        /* DOM renderer remains */
      }
    }
    // else: xterm's built-in DOM renderer, where decorations align pixel-exact.
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
    // The buffer line where output begins (the 133;C exec-start row), captured so
    // Copy output / as-Markdown can read the block's output rows from the buffer.
    let execStartLine: number | undefined;

    // Finished blocks, tracked for the hover tint + the right-click menu. A marker
    // auto-disposes when its line is trimmed from scrollback; disposed entries are
    // skipped + the list is capped so it cannot grow without bound over a long
    // session. [`BlockEntry`] is lifted to component scope so the menu can target
    // the hovered block.
    const blocks: BlockEntry[] = [];
    const MAX_TRACKED = 500;
    let hoverDeco: IDecoration | undefined;
    let hovered: BlockEntry | undefined;

    function onPromptStart(): void {
      // A new block begins here; remember its prompt row so the result strip can
      // anchor to it when the command ends.
      const marker = t.registerMarker(0) ?? undefined;
      promptMarker = marker;
      execStartMs = undefined;
      execStartLine = undefined;
    }

    function onCommandEnd(data: string): void {
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
        const entry: BlockEntry = {
          promptMarker,
          outputStartLine: execStartLine,
          endLine: t.buffer.active.baseY + t.buffer.active.cursorY,
          isError: exitCode !== null && exitCode !== 0,
        };
        result?.onRender((el) => {
          // Keep the live element so hover can toggle `is-hover` (run-again).
          entry.resultEl = el;
          renderBlockResult(el, {
            exitCode,
            durationMs,
            // Run-again replays the engine's validated command as a PTY write
            // (never a webview-decoded 633;E). Inert until the command is
            // fetched, and for a command-less prompt block.
            onRerun: () => {
              const cmd = entry.command;
              if (cmd && cmd.trim().length > 0) {
                void terminalInput(sessionId, `${cmd}\n`);
              }
            },
          });
        });
        blocks.push(entry);
        while (blocks.length > MAX_TRACKED) blocks.shift();
        // Capture the just-finished command from the engine's validated block
        // record for run-again. The engine parses 133;D from its own raw stream
        // and records the block BEFORE it signals the frame this handler drained,
        // so the latest engine block is this one. The light `terminalLastCommand`
        // reads just that one command (no O(history) block re-serialise).
        // Best-effort: a fetch failure leaves run-again inert for this block.
        void terminalLastCommand(sessionId)
          .then((cmd) => {
            entry.command = cmd ?? undefined;
          })
          .catch(() => {});
      }
      promptMarker = undefined;
      execStartMs = undefined;
      execStartLine = undefined;
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
      else if (mark === "exec-start") {
        execStartMs = Date.now();
        // Output begins on the row past the command echo (the current cursor row).
        execStartLine = t.buffer.active.baseY + t.buffer.active.cursorY;
      } else if (mark === "command-end") onCommandEnd(data);
      // Return false so xterm's other handlers still run; the engine parses its
      // own raw copy of the PTY stream, so this never starves its block parser.
      return false;
    };
    t.parser.registerOscHandler(133, dispatch);
    t.parser.registerOscHandler(633, dispatch);

    // Hover tint (arlen-ui's settled look): the block frame is invisible at rest
    // and washes a block's rows when the pointer is over them, revealing the
    // run-again affordance on the result strip. The coder piece is the
    // pointer -> block-row mapping: from the pointer Y, the row under it in the
    // xterm screen, offset by the scroll position to an absolute buffer line,
    // then the tracked block whose [promptMarker.line, endLine] spans it.
    function rowUnderPointer(clientY: number): number | null {
      const screen = t.element?.querySelector(".xterm-screen") as HTMLElement | null;
      if (!screen) return null;
      const rect = screen.getBoundingClientRect();
      const cellH = rect.height / t.rows;
      if (cellH <= 0 || clientY < rect.top || clientY > rect.bottom) return null;
      const viewportRow = Math.floor((clientY - rect.top) / cellH);
      return t.buffer.active.viewportY + viewportRow;
    }

    // The hover tint's own transient marker (distinct from the block's permanent
    // prompt marker): re-created each paint so the anchor can track the viewport,
    // disposed with the decoration so neither leaks.
    let hoverMarker: IMarker | undefined;

    function disposeHoverDeco(): void {
      hoverDeco?.dispose();
      hoverDeco = undefined;
      hoverMarker?.dispose();
      hoverMarker = undefined;
    }

    function clearHover(): void {
      disposeHoverDeco();
      hovered?.resultEl?.classList.remove("is-hover");
      hovered = undefined;
      menuBlock = undefined;
    }

    // Paint (or repaint) the hovered block's tint, clamped to its topmost VISIBLE
    // row. xterm does not render a decoration whose marker row is above the
    // viewport, so anchoring at the prompt marker loses the tint once the command
    // row scrolls off while the output is still on screen (Tim metal). Clamp the
    // anchor to the viewport top and shrink the height to the visible span; if the
    // block is fully scrolled out, paint nothing.
    function paintHover(block: BlockEntry): void {
      disposeHoverDeco();
      if (block.promptMarker.isDisposed) return;
      const viewTop = t.buffer.active.viewportY;
      const viewBottom = viewTop + t.rows - 1;
      // NO +1 on endLine: the end cursor sits past the trailing newline, so
      // [promptMarker.line, endLine] is exactly the block's own rows.
      const top = Math.max(block.promptMarker.line, viewTop);
      const bottom = Math.min(block.endLine, viewBottom);
      if (bottom < top) return; // block fully out of view
      const cursorLine = t.buffer.active.baseY + t.buffer.active.cursorY;
      hoverMarker = t.registerMarker(top - cursorLine) ?? undefined;
      if (!hoverMarker) return;
      hoverDeco =
        t.registerDecoration({
          marker: hoverMarker,
          x: 0,
          width: t.cols,
          height: Math.max(1, bottom - top + 1),
        }) ?? undefined;
      hoverDeco?.onRender((el) => applyBlockHover(el, { isError: block.isError }));
    }

    function setHover(block: BlockEntry): void {
      hovered = block;
      // The hovered block is the right-click menu's target (spec: the hover tint
      // marks it).
      menuBlock = block;
      paintHover(block);
      block.resultEl?.classList.add("is-hover");
    }

    host.addEventListener("pointermove", (e) => {
      const line = rowUnderPointer(e.clientY);
      const block =
        line === null
          ? undefined
          : blocks.find(
              (b) =>
                !b.promptMarker.isDisposed &&
                line >= b.promptMarker.line &&
                line <= b.endLine,
            );
      if (block === hovered) return; // same block (or still none): nothing to do
      clearHover();
      if (block) setHover(block);
    });
    host.addEventListener("pointerleave", () => clearHover());

    // Re-anchor the tint as the view scrolls: the prompt marker may scroll above
    // the viewport while the block's output stays visible, so repaint the hovered
    // block clamped to the new visible span (item 7).
    t.onScroll(() => {
      if (hovered) paintHover(hovered);
    });
  }

  // ── Block right-click menu actions (item 6) ──────────────────────────────
  // The kit ContextMenu look is arlen-ui's (BlockContextMenu.svelte); these wire
  // its handlers from the hovered block's validated record + the xterm buffer. No
  // new backend: copy/select/replay are all local. saveOutput (a file dialog) and
  // the two AI entries (Explain -> ai-explanation, Ask -> the harness @-mention)
  // need cross-app plumbing and are the next slice, so they stay unwired (the menu
  // renders them inert).
  function copyText(text: string): void {
    void navigator.clipboard?.writeText(text).catch(() => {});
  }

  // The block's output rows, read from the xterm buffer between exec-start and the
  // command-end line (trailing blank lines trimmed). Empty for a command-less block.
  function blockOutput(b: BlockEntry): string {
    if (!term || b.outputStartLine === undefined) return "";
    const buf = term.buffer.active;
    const lines: string[] = [];
    for (let i = b.outputStartLine; i <= b.endLine; i++) {
      lines.push(buf.getLine(i)?.translateToString(true) ?? "");
    }
    return lines.join("\n").replace(/\s+$/, "");
  }

  const blockActions = {
    runAgain: () => {
      const c = menuBlock?.command;
      if (c && c.trim().length > 0) void terminalInput(sessionId, `${c}\n`);
    },
    copyCommand: () => {
      if (menuBlock?.command) copyText(menuBlock.command);
    },
    copyOutput: () => {
      if (menuBlock) copyText(blockOutput(menuBlock));
    },
    copyBoth: () => {
      if (menuBlock) copyText(`${menuBlock.command ?? ""}\n${blockOutput(menuBlock)}`.trim());
    },
    copyMarkdown: () => {
      if (menuBlock) {
        copyText(`\`\`\`sh\n${menuBlock.command ?? ""}\n${blockOutput(menuBlock)}\n\`\`\``);
      }
    },
    editRerun: () => {
      // The command back onto the live prompt line, NOT executed (no newline).
      const c = menuBlock?.command;
      if (c) void terminalInput(sessionId, c);
    },
    selectBlock: () => {
      if (term && menuBlock && !menuBlock.promptMarker.isDisposed) {
        term.selectLines(menuBlock.promptMarker.line, menuBlock.endLine);
      }
    },
    saveOutput: () => {
      // First cut: write to a timestamped file (~/Downloads, else $HOME). A
      // save-as dialog is the later enhancement (reuses the same write).
      if (menuBlock) void terminalSaveOutput(blockOutput(menuBlock)).catch(() => {});
    },
  };

  onMount(() => {
    const t = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      // Focus-aware: a hollow cursor when the grid is not focused.
      cursorInactiveStyle: "outline",
      // Off so a right-click opens the block menu instead of selecting a word
      // (word-select stays on double-click); the block menu's target is the
      // hovered block.
      rightClickSelectsWord: false,
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

<BlockContextMenu actions={blockActions}>
  <div class="terminal-host" bind:this={host}></div>
</BlockContextMenu>
