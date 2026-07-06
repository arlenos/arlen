<script lang="ts">
  /// The screenshot annotate surface (SC-R2). A captured image on one canvas with
  /// a floating tool palette; annotate directly on it, then copy on Enter or save.
  /// Satty/Flameshot model, on the @arlen/ui-kit tool archetype, flat house style.
  ///
  /// Mock-vs-live: the annotation is real (canvas). The image source + the
  /// copy/save destinations are the coder's Tauri commands (capture_* / write_png /
  /// clipboard, wrapping sdk/screen-capture); a synthetic fixture stands in under
  /// vite, and copy/save fall back to the browser so the surface is verifiable.
  import { onMount } from "svelte";
  import {
    MousePointer2,
    Crop,
    ArrowUpRight,
    Square,
    Circle,
    Type,
    Pencil,
    Highlighter,
    SquareDashedBottom,
    ListOrdered,
    Undo2,
    Redo2,
    Copy,
    Download,
  } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { drawShape, rectOf, type Shape, type ShapeKind, type ToolKind, type Point } from "$lib/annotate";

  const TOOLS: { kind: ToolKind; label: string; icon: typeof Crop; key: string }[] = [
    { kind: "select", label: "Select", icon: MousePointer2, key: "V" },
    { kind: "crop", label: "Crop", icon: Crop, key: "C" },
    { kind: "arrow", label: "Arrow", icon: ArrowUpRight, key: "A" },
    { kind: "box", label: "Box", icon: Square, key: "R" },
    { kind: "ellipse", label: "Ellipse", icon: Circle, key: "E" },
    { kind: "text", label: "Text", icon: Type, key: "T" },
    { kind: "pen", label: "Pen", icon: Pencil, key: "P" },
    { kind: "highlight", label: "Highlighter", icon: Highlighter, key: "H" },
    { kind: "blur", label: "Blur / redact", icon: SquareDashedBottom, key: "B" },
    { kind: "number", label: "Step", icon: ListOrdered, key: "N" },
  ];
  // The annotation palette is the house semantic set (there is no house blue);
  // resolved from the tokens on mount so the canvas gets concrete hex.
  const SWATCH_TOKENS = ["--color-error", "--color-warning", "--color-success", "--color-fg-primary", "--color-fg-inverse"];
  let swatches = $state<{ token: string; hex: string }[]>([]);
  const SIZES = [
    { v: 2, label: "Thin", dot: 4 },
    { v: 4, label: "Medium", dot: 7 },
    { v: 6, label: "Thick", dot: 10 },
  ];

  let tool = $state<ToolKind>("arrow");
  let color = $state("#ef4444");
  let size = $state(4);
  let shapes = $state<Shape[]>([]);
  let redoStack = $state<Shape[]>([]);
  let stepN = $state(1);

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let base: HTMLCanvasElement; // the untouched captured image, for redraw + blur
  let draft: Shape | null = null;
  let drawing = false;

  // A text box being typed, positioned over the canvas.
  let textEdit = $state<{ x: number; y: number; value: string } | null>(null);

  let seq = 0;

  onMount(() => {
    const cs = getComputedStyle(document.documentElement);
    swatches = SWATCH_TOKENS.map((t) => ({ token: t, hex: (cs.getPropertyValue(t).trim() || "#ffffff") }));
    color = swatches[0]?.hex ?? color;
    base = buildFixture();
    ctx = canvas.getContext("2d");
    canvas.width = base.width;
    canvas.height = base.height;
    redraw();
  });

  function redraw() {
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(base, 0, 0);
    for (const s of shapes) drawShape(ctx, s, base);
    if (draft) drawShape(ctx, draft, base);
  }

  function toCanvas(e: PointerEvent): Point {
    const r = canvas.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * canvas.width, y: ((e.clientY - r.top) / r.height) * canvas.height };
  }

  function onDown(e: PointerEvent) {
    const p = toCanvas(e);
    if (tool === "text") {
      commitText();
      const r = canvas.getBoundingClientRect();
      textEdit = { x: e.clientX - r.left, y: e.clientY - r.top, value: "" };
      queueMicrotask(() => textArea?.focus());
      return;
    }
    if (tool === "number") {
      push({ id: ++seq, kind: "number", color, size, start: p, end: p, n: stepN++ });
      return;
    }
    if (tool === "select" || tool === "crop") return;
    drawing = true;
    // Capture keeps the drag alive if the pointer leaves the canvas; a failure
    // (no active pointer on some inputs) must not abort the draw.
    try {
      canvas.setPointerCapture(e.pointerId);
    } catch {
      /* capture unavailable; drawing still tracks via the window */
    }
    draft = { id: ++seq, kind: tool as ShapeKind, color, size, start: p, end: p, points: [p] };
  }

  function onMove(e: PointerEvent) {
    if (!drawing || !draft) return;
    const p = toCanvas(e);
    draft.end = p;
    if (draft.kind === "pen" || draft.kind === "highlight") draft.points?.push(p);
    redraw();
  }

  function onUp() {
    if (!drawing || !draft) return;
    drawing = false;
    const s = draft;
    draft = null;
    const r = rectOf(s.start, s.end);
    // Drop a zero-size accidental click (except pen, which has a path).
    if (s.kind !== "pen" && s.kind !== "highlight" && r.w < 3 && r.h < 3) {
      redraw();
      return;
    }
    push(s);
  }

  function push(s: Shape) {
    shapes = [...shapes, s];
    redoStack = [];
    redraw();
  }
  function undo() {
    if (shapes.length === 0) return;
    redoStack = [...redoStack, shapes[shapes.length - 1]];
    shapes = shapes.slice(0, -1);
    redraw();
  }
  function redo() {
    if (redoStack.length === 0) return;
    shapes = [...shapes, redoStack[redoStack.length - 1]];
    redoStack = redoStack.slice(0, -1);
    redraw();
  }

  let textArea: HTMLTextAreaElement | null = $state(null);
  function commitText() {
    if (textEdit && textEdit.value.trim()) {
      const r = canvas.getBoundingClientRect();
      const p = { x: (textEdit.x / r.width) * canvas.width, y: (textEdit.y / r.height) * canvas.height };
      push({ id: ++seq, kind: "text", color, size, start: p, end: p, text: textEdit.value });
    }
    textEdit = null;
  }

  async function copy() {
    commitText();
    canvas.toBlob(async (blob) => {
      if (!blob) return;
      try {
        // Live: the coder's clipboard command over the captured PNG. Fallback: the
        // browser clipboard so the affordance works under vite.
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      } catch {
        /* no clipboard permission in the harness */
      }
    }, "image/png");
  }
  async function save() {
    commitText();
    // Live: invoke("save_to_file", ...) over the coder's write_png bridge. Under
    // vite, download the composed PNG so the flow is verifiable.
    canvas.toBlob((blob) => {
      if (!blob) return;
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = fileName();
      a.click();
      URL.revokeObjectURL(url);
    }, "image/png");
  }
  function fileName(): string {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, "0");
    return `Screenshot-${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}.png`;
  }

  function onKey(e: KeyboardEvent) {
    if (textEdit) {
      if (e.key === "Escape") { textEdit = null; }
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") { e.preventDefault(); e.shiftKey ? redo() : undo(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") { e.preventDefault(); copy(); return; }
    if (e.key === "Enter") { e.preventDefault(); copy(); return; }
    const t = TOOLS.find((x) => x.key.toLowerCase() === e.key.toLowerCase());
    if (t) tool = t.kind;
  }

  // A synthetic captured image so the surface renders + verifies without the
  // compositor: a desktop-ish scene with a card and a line worth redacting.
  function buildFixture(): HTMLCanvasElement {
    const c = document.createElement("canvas");
    c.width = 1200;
    c.height = 750;
    const x = c.getContext("2d")!;
    const g = x.createLinearGradient(0, 0, 1200, 750);
    g.addColorStop(0, "#1b2233");
    g.addColorStop(1, "#0e1420");
    x.fillStyle = g;
    x.fillRect(0, 0, 1200, 750);
    // a window card
    x.fillStyle = "#161b26";
    roundRect(x, 180, 130, 840, 500, 16);
    x.fill();
    x.fillStyle = "#1e2532";
    roundRect(x, 180, 130, 840, 46, 16);
    x.fill();
    x.fillStyle = "#e6e8ee";
    x.font = '600 22px "Inter Variable", system-ui, sans-serif';
    x.fillText("Account", 220, 210);
    x.fillStyle = "#9aa4b2";
    x.font = '16px "Inter Variable", system-ui, sans-serif';
    x.fillText("Signed in as", 220, 262);
    x.fillStyle = "#e6e8ee";
    x.font = '500 18px "Inter Variable", system-ui, sans-serif';
    x.fillText("tim@example.com   ·   token: sk-9f2c1a7b4e88", 220, 292);
    x.fillStyle = "#9aa4b2";
    x.font = '16px "Inter Variable", system-ui, sans-serif';
    for (const [i, line] of ["Recent activity", "Opened three files this morning.", "Synced the project to the cloud."].entries()) {
      x.fillText(line, 220, 360 + i * 34);
    }
    return c;
  }
  function roundRect(x: CanvasRenderingContext2D, rx: number, ry: number, w: number, h: number, r: number) {
    x.beginPath();
    x.moveTo(rx + r, ry);
    x.arcTo(rx + w, ry, rx + w, ry + h, r);
    x.arcTo(rx + w, ry + h, rx, ry + h, r);
    x.arcTo(rx, ry + h, rx, ry, r);
    x.arcTo(rx, ry, rx + w, ry, r);
    x.closePath();
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="tool">
  <div class="stage">
    <div class="canvas-wrap">
      <canvas
        bind:this={canvas}
        class="board"
        class:crosshair={tool !== "select"}
        onpointerdown={onDown}
        onpointermove={onMove}
        onpointerup={onUp}
      ></canvas>
      {#if textEdit}
        <textarea
          bind:this={textArea}
          class="text-input"
          style={`left:${textEdit.x}px; top:${textEdit.y}px; color:${color}; font-size:${12 + size * 4}px`}
          bind:value={textEdit.value}
          onblur={commitText}
          rows="1"
          placeholder="Type…"
        ></textarea>
      {/if}
    </div>
  </div>

  <div class="palette">
    {#each TOOLS as t (t.kind)}
      <Button variant={tool === t.kind ? "secondary" : "ghost"} size="icon-sm" title={`${t.label} (${t.key})`} aria-label={t.label} onclick={() => (tool = t.kind)}>
        <t.icon size={16} strokeWidth={1.75} />
      </Button>
    {/each}

    <span class="sep" aria-hidden="true"></span>

    <div class="swatches">
      {#each swatches as s (s.token)}
        <button class="swatch" class:active={color === s.hex} style={`background:${s.hex}`} aria-label={`Colour ${s.token.replace("--color-", "")}`} onclick={() => (color = s.hex)}></button>
      {/each}
    </div>

    {#each SIZES as sz (sz.v)}
      <Button variant={size === sz.v ? "secondary" : "ghost"} size="icon-sm" title={sz.label} aria-label={sz.label} onclick={() => (size = sz.v)}>
        <span class="size-bar" style={`height:${sz.v}px`}></span>
      </Button>
    {/each}

    <span class="sep" aria-hidden="true"></span>

    <Button variant="ghost" size="icon-sm" title="Undo (Ctrl+Z)" aria-label="Undo" disabled={shapes.length === 0} onclick={undo}><Undo2 size={16} strokeWidth={1.75} /></Button>
    <Button variant="ghost" size="icon-sm" title="Redo (Ctrl+Shift+Z)" aria-label="Redo" disabled={redoStack.length === 0} onclick={redo}><Redo2 size={16} strokeWidth={1.75} /></Button>

    <span class="sep" aria-hidden="true"></span>

    <Button variant="outline" size="sm" title="Copy (Enter)" onclick={copy}><Copy size={15} strokeWidth={1.75} /> Copy</Button>
    <Button variant="default" size="sm" title="Save" onclick={save}><Download size={15} strokeWidth={1.75} /> Save</Button>
  </div>
</div>

<style>
  .tool {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-shell, #0a0a0a);
    color: var(--foreground, #fafafa);
    overflow: hidden;
  }
  .stage {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }
  .canvas-wrap {
    position: relative;
    max-width: 100%;
    max-height: 100%;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
    border-radius: var(--radius-card, 12px);
    overflow: hidden;
  }
  .board {
    display: block;
    max-width: 100%;
    max-height: calc(100vh - 8rem);
    width: auto;
    height: auto;
    object-fit: contain;
  }
  .board.crosshair {
    cursor: crosshair;
  }
  .text-input {
    position: absolute;
    min-width: 6rem;
    border: none;
    outline: 1px dashed color-mix(in srgb, currentColor 60%, transparent);
    background: transparent;
    font-family: "Inter Variable", system-ui, sans-serif;
    font-weight: 600;
    line-height: 1.25;
    resize: none;
    overflow: hidden;
    padding: 0;
  }

  /* The floating tool palette: one flat bar on the house surface tokens. */
  .palette {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    align-self: center;
    margin-bottom: 1rem;
    padding: 0.375rem 0.5rem;
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
  }
  .sep {
    width: 1px;
    height: 1.5rem;
    margin: 0 0.25rem;
    background: var(--color-border);
  }
  .swatches {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0 0.125rem;
  }
  .swatch {
    width: 1.125rem;
    height: 1.125rem;
    border-radius: var(--radius-button);
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 25%, transparent);
    cursor: pointer;
    padding: 0;
  }
  .swatch.active {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .size-bar {
    display: block;
    width: 14px;
    border-radius: 1px;
    background: currentColor;
  }
</style>
