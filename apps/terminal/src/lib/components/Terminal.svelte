<script lang="ts">
  // The real terminal grid: an xterm.js instance fed the raw PTY byte stream
  // (terminal.md re-architecture, Tim: engine-down, renderer-out). The Rust
  // engine pumps raw bytes; xterm.js owns the VT parsing + render, so the grid
  // is a GPU/canvas surface themed by a palette - NOT the DOM-span-per-cell
  // render. The block frame, inline images and artifacts stay web-UI around
  // this; only the live grid is xterm.js.
  import { onMount, onDestroy } from "svelte";
  import { Terminal, type IMarker, type IDecoration, type ILink } from "@xterm/xterm";
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
    terminalInjectBlock,
    terminalSaveOutput,
    terminalConfigGet,
    terminalConfigSet,
    openUrl,
  } from "$lib/contract";
  import { linksOnRow } from "$lib/links";
  import { matchZoom, zoomStep, type ZoomAction } from "$lib/zoom";
  import { get } from "svelte/store";
  // Aliased: `t` is the local xterm instance throughout the mount, and the store
  // would shadow it inside exactly the callback that needs both.
  import { t as messages } from "$lib/i18n/messages";
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
  import { commandAmbient, publishAmbient } from "$lib/commandAmbient";
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
  /// True while the engine is refusing writes, so the message is written once per
  /// outage rather than once per keystroke.
  let inputRefused = false;
  let fit: FitAddon | undefined;
  let unlistenFrame: UnlistenFn | undefined;
  let unlistenA11y: UnlistenFn | undefined;
  let resizeObserver: ResizeObserver | undefined;
  // The persisted base font size (terminal config, §5b); zoom is a transient
  // delta over it and Ctrl+0 resets back to it. Loaded on mount, the const is
  // the pre-config fallback.
  let baseFontSize = TERMINAL_FONT_SIZE;

  // Apply a zoom shortcut to the live grid: change the font size, re-fit so the
  // grid + PTY reflow, and keep the new size (see persistZoom below).
  function applyZoom(action: ZoomAction): void {
    if (!term || !fit) return;
    const current = term.options.fontSize ?? baseFontSize;
    const next = action === "reset" ? baseFontSize : zoomStep(current, action);
    if (next === current) return;
    term.options.fontSize = next;
    fit.fit();
    // Re-pin: the new size has a fractional cell again, so land it back on a whole
    // pixel column (the first fit measured the new face; pin reads it, re-fit applies).
    pinCellWidthToInteger(term);
    fit.fit();
    persistZoom(next);
  }

  /// Whether the size on screen is also the size on disk.
  ///
  /// Said once per outage, not once per keystroke: Ctrl+- held down is a run of
  /// steps and one sentence per step would paint the grid with it. The next
  /// accepted write re-arms the line, the same way a refused keystroke does.
  let zoomUnsaved = false;

  /// Keep the size across restarts.
  ///
  /// The mount READS a saved size and nothing ever wrote one, so every zoom was
  /// forgotten on the next launch while the code that would remember it sat
  /// beside the code that read it. A person who sets their font size and finds it
  /// back at the default reports that as "it forgot", and they are right.
  ///
  /// A refused write says so in the grid, which is where this app says
  /// everything: the size on screen is correct either way, so the fact worth
  /// carrying is the one the screen cannot show - that it will not survive.
  function persistZoom(size: number): void {
    terminalConfigSet(size).then(
      () => {
        zoomUnsaved = false;
      },
      () => {
        if (zoomUnsaved || !term) return;
        zoomUnsaved = true;
        term.write(`\r\n\x1b[33m${get(messages)("term.zoomNotSaved")}\x1b[0m\r\n`);
      },
    );
  }

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
        // The desktop's slow pulse for a command that outlasts somebody's
        // patience. Armed here and taken down at command-end; see
        // `commandAmbient.ts` for why it waits rather than firing on every
        // command.
        ambientDriver.started();
        execStartMs = Date.now();
        // Output begins on the row past the command echo (the current cursor row).
        execStartLine = t.buffer.active.baseY + t.buffer.active.cursorY;
      } else if (mark === "command-end") {
        ambientDriver.ended();
        onCommandEnd(data);
      }
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
  // its handlers from the hovered block's validated record + the xterm buffer.
  // copy/select/replay/saveOutput are local; `ask` crosses to the assistant.
  //
  // `ask` was left UNWIRED - and so rendered disabled - on the grounds that the
  // harness had no "inject scoped context" receiver. It has had one since 27
  // August: the terminal writes a 0600 one-shot payload and launches
  // `arlen-harness --inject <path>`, and the harness reads that arg on launch.
  // Both halves were built and the note that kept the entry dead was not. That is
  // the cost of a reason written as a description - it goes on being read as a
  // verdict after the thing it describes has ended. (Explain was dropped,
  // terminal.md §4.11.)
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
    ask: () => {
      // The payload IS the grant: the person chose this block, it goes over as a
      // scoped @-mention, and the harness reads it advisorily. Pull-only, no
      // silent graph write.
      if (!menuBlock) return;
      void terminalInjectBlock(menuBlock.command ?? "", blockOutput(menuBlock)).catch(
        (e: unknown) => {
          // Two different facts, and only one of them is worth trying again. The
          // backend answers with a marker so the sentence can be translated here;
          // an unrecognised error is the second case, since a machine that HAS the
          // assistant is the ordinary one.
          const key = String(e).includes("harness-not-installed")
            ? "term.askNoAssistant"
            : "term.askFailed";
          term?.write(`\r\n\x1b[31m${get(messages)(key)}\x1b[0m\r\n`);
        },
      );
    },
    saveOutput: () => {
      // First cut: write to a timestamped file (~/Downloads, else $HOME). A
      // save-as dialog is the later enhancement (reuses the same write).
      //
      // BOTH outcomes go into the grid, which is the only surface this menu has.
      // The command answers with the path it chose and the menu was throwing it
      // away along with the failure, so a person pressing Save got the same
      // nothing either way and no idea where to look for a file that may not
      // exist. `get`, not `$t`: this runs in a menu callback.
      if (!menuBlock) return;
      void terminalSaveOutput(blockOutput(menuBlock)).then(
        (path) => {
          term?.write(`\r\n\x1b[32m${get(messages)("term.saved", { path })}\x1b[0m\r\n`);
        },
        () => {
          term?.write(`\r\n\x1b[31m${get(messages)("term.saveFailed")}\x1b[0m\r\n`);
        },
      );
    },
  };

  /// Open a URL in the user's browser, and say so in the log when it will not go.
  ///
  /// A refused open is not something the person who clicked can act on: the
  /// scheme allowlist is the same on both sides, so a rejection here means the
  /// scanner offered something the opener does not take. That is ours to fix,
  /// not theirs to read.
  function open(url: string): void {
    openUrl(url).catch((e) => console.warn("terminal: open_url refused", url, e));
  }

  /// Make URLs in the output clickable.
  ///
  /// TWO PATHS, because two different things arrive as a link. A program that
  /// speaks OSC 8 hands xterm the URL itself and xterm finds it without help -
  /// but with no `linkHandler` set, xterm's own fallback puts an English
  /// `confirm()` box on screen and then calls `window.open`, which in a window
  /// with no chrome is a dialogue nobody can read in their language followed by
  /// a page with no way back. The handler replaces both halves of that.
  ///
  /// The provider is the other path: a bare URL printed by a program that knows
  /// nothing about hyperlinks, which is nearly all of them. It scans the whole
  /// logical line rather than the row under the pointer, so a long address split
  /// across two rows is one link instead of a click on its first half.
  function registerLinks(t: Terminal): void {
    t.options.linkHandler = {
      activate: (_event, text) => open(text),
    };
    t.registerLinkProvider({
      // The whole body is `linksOnRow`, which lives beside the pure halves it
      // joins so a test can reach it: the row-numbering and range off-by-ones are
      // in the joining, and neither a unit test of the parts nor a drive could
      // see them - the WebGL renderer draws link underlines into the canvas.
      provideLinks(bufferLineNumber, callback) {
        callback(linksOnRow(t.buffer.active, bufferLineNumber, open) as ILink[] | undefined);
      },
    });
  }

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
    registerLinks(t);

    // Zoom shortcuts (§5b): Ctrl +/-/0 adjust the font size and must NOT reach the
    // PTY (Ctrl+- is a control byte to the shell otherwise). A non-zoom key is
    // passed through (return true) so xterm handles it normally.
    t.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const action = matchZoom(e);
      if (!action) return true;
      // Swallow it from the PTY AND the webview's own Ctrl+/-/0 zoom.
      e.preventDefault();
      applyZoom(action);
      return false;
    });

    // The grid IS the keystroke target now (not a textbox): xterm.js emits the
    // UTF-8 input string, the engine writes it to the PTY master.
    //
    // A refused write says so IN THE GRID, which is where everything else in a
    // terminal is said. It used to be `void terminalInput(...)`: the promise
    // rejected into nothing, so a dead PTY swallowed every keystroke and the
    // window looked like a terminal that had stopped caring.
    //
    // Once per outage, not once per keystroke. `onData` fires per character, so
    // reporting each one would paint the screen with the same sentence while
    // someone types - the noise version of the same defect. The next accepted
    // write re-arms it.
    t.onData((d) => {
      terminalInput(sessionId, d).then(
        () => {
          inputRefused = false;
        },
        () => {
          if (inputRefused || !term) return;
          inputRefused = true;
          // `get`, not `$t`: this runs inside a callback, and a store
          // subscription there is not a top-level one.
          term.write(`\r\n\x1b[31m${get(messages)("term.notAccepted")}\x1b[0m\r\n`);
        },
      );
    });
    // When xterm.js recomputes the geometry (on fit), resize the PTY to match so
    // the shell + TUIs reflow. Registered BEFORE the first fit() below: the
    // initial fit emits a resize, and if its handler is not yet attached that
    // event is lost and the PTY stays at its 80x24 spawn size - an alt-screen
    // TUI then draws only 24 rows into a taller grid (the under-fill bug).
    t.onResize(({ cols, rows }) => void terminalResize(sessionId, cols, rows));

    void listen<string>("terminal://frame", (e) => {
      if (e.payload === sessionId) void drain();
    }).then((un) => (unlistenFrame = un));

    // The session's screen-reader flag, published by the shell from the config
    // broker. xterm paints the grid on a canvas, which a screen reader cannot
    // read at all; `screenReaderMode` makes it keep a hidden live-region mirror
    // of the visible rows instead, so the terminal has something to announce.
    //
    // It is off by default because the mirror costs DOM writes on every render,
    // and driven by the session flag rather than a per-app setting: somebody who
    // needs it needs it everywhere, and should not have to find the switch again
    // in each app. The host subscribes to a retained `.state` topic, so the
    // first event arrives whether or not anything has changed since login.
    void listen<boolean>("arlen://accessibility-changed", (e) => {
      t.options.screenReaderMode = e.payload;
    }).then((un) => (unlistenA11y = un));

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
    void document.fonts.ready.then(async () => {
      if (!term || !fit) return;
      // Apply the persisted base font size (§5b) before sizing the grid - the
      // const was only the pre-config fallback. A failed/absent read keeps it.
      try {
        const cfg = await terminalConfigGet();
        if (Number.isFinite(cfg.font_size) && cfg.font_size > 0) {
          baseFontSize = cfg.font_size;
          if (term) term.options.fontSize = cfg.font_size;
        }
      } catch {
        /* keep the fallback size */
      }
      if (!term || !fit) return;
      fit.fit();
      // Pin the cell to a whole css pixel so every column lands on an integer pixel
      // (see pinCellWidthToInteger). The first fit measured the loaded face; pin,
      // re-fit, and repeat once with a frame between so xterm re-measures the scaled
      // face. The second pass is a no-op unless the engine left a sub-pixel residual,
      // in which case it converges. Bounded so it can never spin.
      for (let pass = 0; pass < 2; pass++) {
        const changed = pinCellWidthToInteger(term);
        fit.fit();
        if (!changed) break;
        await new Promise((resolve) => requestAnimationFrame(resolve));
        if (!term || !fit) return;
      }
      resizeObserver = new ResizeObserver(() => fit?.fit());
      resizeObserver.observe(host);
      void drain();
    });
  });

  // Pin xterm's cell to a WHOLE css pixel by nudging the font size, so every column
  // lands on an integer pixel column. xterm sizes each cell to its measured glyph
  // advance (Cascadia is ~8.4px at 14px, a fraction); at a fractional device-pixel
  // ratio (the display here is 1.5x) the browser snaps each cell to a whole device
  // pixel, so consecutive cells land 8 or 9px apart - a wide TUI's box rule breaks
  // and its right border doubles (item 8, btop in full width). letterSpacing cannot
  // fix this: xterm rounds it to whole pixels and ADDS it to the fractional advance,
  // so it can never produce an integer cell (verified headless). The font size can:
  // xterm's css cell width is round(advance * cols) / cols, so an integer advance
  // makes the css cell that same integer for every column count and every dpr.
  // Scaling the size by round(advance)/advance lands the advance on the nearest
  // whole pixel while the glyphs scale with the cell, so the box rule stays
  // continuous (the near-zero letter-spacing xterm applies stays near zero, no
  // gaps). Verified headless at dpr 1.0 and 1.5: the css cell goes 8.39873 -> 8.0
  // and the full-width box rule stays unbroken. Targets xterm's OWN measured advance
  // (`_charSizeService.width`, the value that drives the css cell, not a separate DOM
  // probe which a font-subset fallback can disagree with); a no-op when that path is
  // absent or the advance is already whole, so it can never break rendering. Returns
  // whether it changed the size so the caller can re-fit and converge any residual.
  function pinCellWidthToInteger(t: Terminal): boolean {
    const advance = (t as unknown as { _core?: { _charSizeService?: { width?: number } } })._core
      ?._charSizeService?.width;
    if (typeof advance !== "number" || !(advance > 0)) return false;
    const target = Math.round(advance);
    if (target < 1 || Math.abs(advance - target) < 0.01) return false;
    const size = t.options.fontSize ?? baseFontSize;
    const next = (size * target) / advance;
    if (!Number.isFinite(next) || next <= 0) return false;
    t.options.fontSize = next;
    return true;
  }

  // The desktop pulse for a long-running command. One per window: the shell keeps
  // one ambient effect per app, so a second terminal window publishing its own
  // would be the same slot written twice - which is correct behaviour (whichever
  // command is running most recently is the one worth saying) rather than a
  // conflict to resolve here.
  const ambientDriver = commandAmbient(
    publishAmbient,
    (fn, ms) => window.setTimeout(fn, ms),
    (handle) => window.clearTimeout(handle),
  );

  onDestroy(() => {
    // A window that goes away with a command still running would otherwise leave
    // the desktop pulsing for something nobody can see. The shell reclaims it on
    // observed absence too, but only once the window list catches up, and the
    // app knows first.
    ambientDriver.dispose();
    unlistenFrame?.();
    unlistenA11y?.();
    resizeObserver?.disconnect();
    term?.dispose();
  });
</script>

<BlockContextMenu actions={blockActions}>
  <div class="terminal-host" bind:this={host}></div>
</BlockContextMenu>
