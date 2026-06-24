<script lang="ts">
  // The real terminal grid: an xterm.js instance fed the raw PTY byte stream
  // (terminal.md re-architecture, Tim: engine-down, renderer-out). The Rust
  // engine pumps raw bytes; xterm.js owns the VT parsing + render, so the grid
  // is a GPU/canvas surface themed by a palette - NOT the DOM-span-per-cell
  // render. The block frame, inline images and artifacts stay web-UI around
  // this; only the live grid is xterm.js.
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { CanvasAddon } from "@xterm/addon-canvas";
  import "@xterm/xterm/css/xterm.css";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { terminalDrainOutput, terminalInput, terminalResize } from "$lib/contract";

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
  // way): the shell's OSC 133 marks delimit commands, so on each prompt-start
  // (133;A) we drop an xterm.js marker + a left-accent decoration. The grid stays
  // one canvas - the block frame is an overlay anchored to a row, not a DOM grid.
  // This is the coder-owned anchor mechanism; the richer frame (header, run-again)
  // is arlen-ui's lane layered on these marks.
  function registerBlockMarks(t: Terminal): void {
    t.parser.registerOscHandler(133, (data: string) => {
      // 133;A = prompt start: a new command block begins at this row.
      if (data === "A" || data.startsWith("A;")) {
        const marker = t.registerMarker(0);
        if (marker) {
          const dec = t.registerDecoration({ marker, width: 1, x: 0 });
          dec?.onRender((el: HTMLElement) => {
            el.style.borderLeft = "2px solid var(--accent, #6366f1)";
            el.style.height = "100%";
            el.style.boxSizing = "border-box";
          });
        }
      }
      // Never consume the sequence: the engine's own OSC 133 block parsing must
      // still see it. Returning false lets every other handler run.
      return false;
    });
  }

  onMount(() => {
    const t = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      // Focus-aware: a hollow cursor when the grid is not focused.
      cursorInactiveStyle: "outline",
      rightClickSelectsWord: true,
      allowProposedApi: true,
      fontFamily: "monospace",
    });
    term = t;
    fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host);
    loadRenderer(t);
    registerBlockMarks(t);
    fit.fit();

    // The grid IS the keystroke target now (not a textbox): xterm.js emits the
    // UTF-8 input string, the engine writes it to the PTY master.
    t.onData((d) => void terminalInput(sessionId, d));
    // When xterm.js recomputes the geometry (on fit), resize the PTY to match so
    // the shell + TUIs reflow.
    t.onResize(({ cols, rows }) => void terminalResize(sessionId, cols, rows));

    void listen<string>("terminal://frame", (e) => {
      if (e.payload === sessionId) void drain();
    }).then((un) => (unlistenFrame = un));

    resizeObserver = new ResizeObserver(() => fit?.fit());
    resizeObserver.observe(host);

    // xterm.js owns input + focus now (its textarea is the keystroke target), so
    // focus it on mount: the cursor draws solid (not the inactive outline) and
    // keystrokes land without a click. xterm re-focuses on click too.
    t.focus();

    // Drain whatever the engine already buffered before this view attached.
    void drain();
  });

  onDestroy(() => {
    unlistenFrame?.();
    resizeObserver?.disconnect();
    term?.dispose();
  });
</script>

<div class="terminal-host" bind:this={host}></div>
