<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import { initAppMenu, menuAction } from "$lib/menu";
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
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import FloatingThumbnail from "$lib/components/FloatingThumbnail.svelte";
  import { drawShape, rectOf, type Shape, type ShapeKind, type ToolKind, type Point } from "$lib/annotate";
  import {
    isTauri,
    capturePrimary,
    captureSources,
    captureOutput,
    captureWindow,
    saveScreenshot,
    copyPng,
    frontendLog,
    canvasPngBase64,
    closeWindow,
    type Output,
    type Window as CaptureWindow,
    type CaptureRefusal,
  } from "$lib/bridge";

  // The capture handoff: a fresh capture floats as a thumbnail (ignore -> auto-save,
  // click -> annotate); the annotate surface stays mounted so its canvas is ready.
  let phase = $state<"thumbnail" | "annotate" | "dismissed">("thumbnail");

  // `label` is a message KEY, resolved with $t where it renders: a top-level
  // const would capture the locale at import and never follow a switch.
  const TOOLS: { kind: ToolKind; label: string; icon: typeof Crop; key: string }[] = [
    { kind: "select", label: "s.tool.select", icon: MousePointer2, key: "V" },
    { kind: "crop", label: "s.tool.crop", icon: Crop, key: "C" },
    { kind: "arrow", label: "s.tool.arrow", icon: ArrowUpRight, key: "A" },
    { kind: "box", label: "s.tool.box", icon: Square, key: "R" },
    { kind: "ellipse", label: "s.tool.ellipse", icon: Circle, key: "E" },
    { kind: "text", label: "s.tool.text", icon: Type, key: "T" },
    { kind: "pen", label: "s.tool.pen", icon: Pencil, key: "P" },
    { kind: "highlight", label: "s.tool.highlight", icon: Highlighter, key: "H" },
    { kind: "blur", label: "s.tool.blur", icon: SquareDashedBottom, key: "B" },
    { kind: "number", label: "s.tool.number", icon: ListOrdered, key: "N" },
  ];
  // The annotation palette is the house semantic set (there is no house blue);
  // resolved from the tokens on mount so the canvas gets concrete hex.
  const SWATCH_TOKENS = ["--color-error", "--color-warning", "--color-success", "--color-fg-primary", "--color-fg-inverse"];
  let swatches = $state<{ token: string; hex: string }[]>([]);
  const SIZES = [
    { v: 2, label: "s.size.thin", dot: 4 },
    { v: 4, label: "s.size.medium", dot: 7 },
    { v: 6, label: "s.size.thick", dot: 10 },
  ];

  let tool = $state<ToolKind>("arrow");
  let color = $state("#ef4444");
  let size = $state(4);
  let shapes = $state<Shape[]>([]);
  let redoStack = $state<Shape[]>([]);
  let stepN = $state(1);

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let base = $state<HTMLCanvasElement>(); // the untouched captured image, for redraw + blur + the thumbnail
  let draft: Shape | null = null;
  let drawing = false;

  /// The rectangle being dragged with the crop tool.
  ///
  /// NOT a `Shape`: `ShapeKind` excludes crop because cropping is not something
  /// drawn onto the picture, it changes what the picture IS. Kept separate for
  /// that reason rather than squeezed into the shape list.
  ///
  /// The tool has been in the palette since the app was written, with an icon
  /// and the `C` shortcut, and `onDown` returned early for it - so selecting it
  /// and dragging did nothing at all. A button that performs no action is the
  /// same lie as a sentence that states no fact, and this one sat next to nine
  /// tools that work.
  let cropDrag = $state<{ start: Point; end: Point } | null>(null);

  // A text box being typed, positioned over the canvas.
  let textEdit = $state<{ x: number; y: number; value: string } | null>(null);

  let seq = 0;

  // The shell menu's dispatch, the same verbs the keys run.
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "edit.undo") undo();
    else if (a === "edit.redo") redo();
    else if (a === "edit.copy") void copy();
  });

  onMount(async () => {
    void initAppMenu();
    const cs = getComputedStyle(document.documentElement);
    swatches = SWATCH_TOKENS.map((t) => ({ token: t, hex: (cs.getPropertyValue(t).trim() || "#ffffff") }));
    color = swatches[0]?.hex ?? color;
    // Live: the coder's capture command hands back the primary output as a PNG
    // data URL. The other two answers are NOT the same answer. Under vite there
    // is no screen to capture and the fixture is honest, labelled as a sample.
    // On a host that cannot capture there IS a screen and we did not get it, so
    // there is nothing to show and nothing to save: inventing a desktop here is
    // how a person ends up sending a picture of a machine that does not exist.
    const captured = await capturePrimary();
    if (captured.kind === "unavailable") {
      captureFailure = captured.why;
      return;
    }
    isSample = captured.kind === "hostless";
    // What else could be photographed, for the picker. Asked after the first
    // capture so the picture arrives first and this never delays it.
    void captureSources().then((s) => {
      outputs = s.outputs;
      windows = s.windows;
    });
    base = captured.kind === "image" ? await dataUrlToCanvas(captured.dataUrl) : buildFixture();
    ctx = canvas.getContext("2d");
    canvas.width = base.width;
    canvas.height = base.height;
    redraw();
  });

  // Load a PNG data URL into an offscreen canvas (the untouched capture base).
  function dataUrlToCanvas(dataUrl: string): Promise<HTMLCanvasElement> {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => {
        const c = document.createElement("canvas");
        c.width = img.naturalWidth;
        c.height = img.naturalHeight;
        c.getContext("2d")!.drawImage(img, 0, 0);
        resolve(c);
      };
      img.onerror = () => reject(new Error("capture image failed to load"));
      img.src = dataUrl;
    });
  }

  function redraw() {
    if (!ctx || !base) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(base, 0, 0);
    for (const s of shapes) drawShape(ctx, s, base);
    if (draft) drawShape(ctx, draft, base);
    if (cropDrag) {
      // Dim everything outside the selection so the kept region is the bright
      // one - the convention every screenshot tool uses, and the thing that
      // makes a drag read as "this part" rather than "a box drawn here".
      const r = rectOf(cropDrag.start, cropDrag.end);
      ctx.save();
      ctx.fillStyle = "rgba(0, 0, 0, 0.5)";
      ctx.beginPath();
      ctx.rect(0, 0, canvas.width, canvas.height);
      ctx.rect(r.x, r.y, r.w, r.h);
      ctx.fill("evenodd");
      ctx.strokeStyle = "rgba(255, 255, 255, 0.9)";
      ctx.lineWidth = 1;
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      ctx.restore();
    }
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
    if (tool === "crop") {
      cropDrag = { start: p, end: p };
      try {
        canvas.setPointerCapture(e.pointerId);
      } catch {
        /* capture unavailable; the drag still tracks */
      }
      return;
    }
    if (tool === "select") return;
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
    if (cropDrag) {
      cropDrag = { start: cropDrag.start, end: toCanvas(e) };
      redraw();
      return;
    }
    if (!drawing || !draft) return;
    const p = toCanvas(e);
    draft.end = p;
    if (draft.kind === "pen" || draft.kind === "highlight") draft.points?.push(p);
    redraw();
  }

  function onUp() {
    if (cropDrag) {
      const r = rectOf(cropDrag.start, cropDrag.end);
      cropDrag = null;
      // A stray click is not a crop. Below this the result would be a few pixels
      // of nothing, and the person meant to put the tool down.
      //
      // Written as "must be at least 8" rather than "must not be under 8" so a
      // NaN rectangle lands in the reject branch. `toCanvas` divides by the
      // canvas's laid-out width, so a canvas with no layout size hands back NaN
      // points, `NaN < 8` is false, and the old form let exactly that case
      // through to `commitCrop`, which sized the canvas to NaN and blanked the
      // picture. The one input worse than too small must not be the one that
      // gets past a smallness check.
      if (!(r.w >= 8 && r.h >= 8)) {
        redraw();
        return;
      }
      commitCrop(r);
      return;
    }
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

  /// Cut the picture down to `r`, keeping what has been drawn on it.
  ///
  /// The annotations move with the picture rather than being thrown away: a
  /// person who arrowed at something and then cropped to it means to keep the
  /// arrow. They are translated by the crop origin, which is why this touches
  /// the shape list at all.
  function commitCrop(r: { x: number; y: number; w: number; h: number }) {
    if (!base) return;
    const cut = document.createElement("canvas");
    cut.width = Math.round(r.w);
    cut.height = Math.round(r.h);
    cut.getContext("2d")!.drawImage(base, -Math.round(r.x), -Math.round(r.y));
    base = cut;
    shapes = shapes.map((s) => ({
      ...s,
      start: { x: s.start.x - r.x, y: s.start.y - r.y },
      end: { x: s.end.x - r.x, y: s.end.y - r.y },
      points: s.points?.map((p) => ({ x: p.x - r.x, y: p.y - r.y })),
    }));
    // A crop is not undoable by the shape stack, so it clears the redo branch
    // rather than leaving entries that would be replayed onto other pixels.
    redoStack = [];
    canvas.width = cut.width;
    canvas.height = cut.height;
    redraw();
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

  /// What to say when the host refused, and nothing else on this surface can.
  ///
  /// A failed save used to reach `frontendLog` alone, which is the app's stdout:
  /// the button did its animation, the file was never written, and the person
  /// kept the only copy on a canvas they were about to close. The log line is
  /// still there for the path and the reason; this is the half a person sees.
  let actionFailed = $state<"save" | "copy" | null>(null);

  /// Why there is no capture, when a host said it could not take one. Set only on
  /// the real-host path: it is a statement about this machine, and the browser
  /// has nothing to say about that.
  /// Why the capture did not happen, as a word. It held the compositor's own
  /// words - `String(e)`, or an English sentence written in the bridge - and the
  /// surface below drew them, so every language got the same English line under a
  /// translated heading.
  let captureFailure = $state<CaptureRefusal | "no-host" | null>(null);

  /// The sentence for each refusal. `no-host` is this page's own: it is what the
  /// browser preview and the render harness are, not something a machine reported.
  const WHY: Record<CaptureRefusal | "no-host", string> = {
    "no-screencopy": "s.why.noScreencopy",
    refused: "s.why.refused",
    "no-host": "s.noHost",
  };

  /// The screens and windows this machine can be asked to photograph.
  ///
  /// `screenshot-capture-plan.md` names the modes: region, window, full-screen,
  /// current-monitor versus all. The backend has had `list_outputs`,
  /// `list_windows`, `capture_window` and `capture_region` all along and no
  /// surface reached them, so the app could take exactly one kind of picture.
  /// This is the picking half; region needs a drag overlay and is separate.
  let outputs = $state<Output[]>([]);
  let windows = $state<CaptureWindow[]>([]);

  /// What is currently on the canvas, so the picker can show which.
  let source = $state<string>("screen:0");

  /// Re-capture from a chosen source and redraw.
  ///
  /// The app still captures the primary output on open - the floating-thumbnail
  /// handoff the plan is built around depends on a picture existing immediately,
  /// so picking is a change of mind rather than a step before the first shot.
  async function pickSource(value: string) {
    const [kind, idx] = value.split(":");
    const n = Number(idx);
    // The identifier travels with the index, because the list and the capture
    // are separate Wayland connections and a window opening in between moves
    // the index onto somebody else's window.
    const shot =
      kind === "window"
        ? await captureWindow(n, windows.find((w) => w.index === n)?.identifier ?? null)
        : await captureOutput(n, outputs.find((o) => o.index === n)?.name ?? null);
    if (shot.kind !== "image") {
      // A source that will not capture is reported where the picture would be,
      // not swallowed: a picker that silently keeps the old image is one that
      // says you photographed something you did not.
      captureFailure = shot.kind === "unavailable" ? shot.why : "no-host";
      return;
    }
    source = value;
    captureFailure = null;
    base = await dataUrlToCanvas(shot.dataUrl);
    canvas.width = base.width;
    canvas.height = base.height;
    shapes = [];
    redraw();
  }

  /// Whether what is on the canvas is a made-up scene rather than your screen.
  /// True only where a sample is the honest answer - no host, so no screen.
  let isSample = $state(false);

  // Copy / save operate on a given canvas - the annotate canvas for the surface,
  // the untouched base for the thumbnail's quick actions.
  async function copyCanvas(c: HTMLCanvasElement) {
    if (isTauri()) {
      // Live: the coder's clipboard command over the annotated PNG.
      try {
        await copyPng(canvasPngBase64(c));
        actionFailed = null;
      } catch (e) {
        frontendLog(`copy failed: ${e}`);
        actionFailed = "copy";
      }
      return;
    }
    // Fallback: the browser clipboard so the affordance works under vite.
    c.toBlob(async (blob) => {
      if (!blob) return;
      try {
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      } catch {
        /* no clipboard permission in the harness */
      }
    }, "image/png");
  }
  /// Write the canvas out, and ANSWER whether it landed.
  ///
  /// It used to return nothing and settle `actionFailed` from inside its own
  /// `.catch`, which reads fine on the annotate surface - the failure line is
  /// right there. The dismiss path could not use it: it fired this, switched to
  /// a phase that says "Saved to Pictures/Screenshots." and closed the window
  /// 2.5 seconds later, so a failed save was announced as a successful one and
  /// then the capture was gone. A caller that is about to make a claim has to be
  /// able to wait for the answer.
  async function saveCanvas(c: HTMLCanvasElement): Promise<boolean> {
    if (isTauri()) {
      // Live: write the annotated PNG to the screenshots dir via the coder's command.
      try {
        const path = await saveScreenshot(canvasPngBase64(c));
        frontendLog(`saved ${path}`);
        actionFailed = null;
        return true;
      } catch (e) {
        frontendLog(`save failed: ${e}`);
        actionFailed = "save";
        return false;
      }
    }
    // Under vite, download the composed PNG so the flow is verifiable. Reported
    // as a success: there is no backend to refuse, and the dismiss path needs an
    // answer rather than a hang.
    c.toBlob((blob) => {
      if (!blob) return;
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = fileName();
      a.click();
      URL.revokeObjectURL(url);
    }, "image/png");
    return true;
  }
  async function copy() {
    commitText();
    await copyCanvas(canvas);
  }
  async function save() {
    commitText();
    await saveCanvas(canvas);
  }
  // Ignoring the thumbnail auto-saves (the fast path); the dismiss button does too.
  /// How long the "saved to" line stays before the window goes away.
  ///
  /// Long enough to read seven words and no longer: this app's whole job is done
  /// by the time it says that, and a capture tool that stays on screen is in the
  /// way of the thing you took a picture of. It used to stay forever - no
  /// titlebar, no close button, no key that closed it.
  const GOODBYE_MS = 2500;

  /// Dismissing the thumbnail keeps the capture: it saves, says so, and goes.
  ///
  /// The saying-so now waits for the saving. Each of the three steps used to be
  /// unconditional, so a refused write still reached a window that read "Saved to
  /// Pictures/Screenshots." and then closed on its own - the capture lost and the
  /// person told the opposite. On a failure the window now stays open on the
  /// annotate surface, where the save-failed line lives and where Copy and Save
  /// are still reachable: a capture that could not be written is exactly when
  /// somebody needs the window that is holding it.
  ///
  /// `base` absent is the same answer. Nothing was captured, so there is nothing
  /// to claim was kept.
  async function autoSaveAndDismiss() {
    if (!base || !(await saveCanvas(base))) {
      actionFailed = "save";
      phase = "annotate";
      return;
    }
    phase = "dismissed";
    setTimeout(() => void closeWindow(), GOODBYE_MS);
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
    // Escape with nothing to cancel means "I am done with this window", which is
    // what a person reaches for on a transient capture tool and what this had no
    // answer to.
    if (e.key === "Escape") { e.preventDefault(); void closeWindow(); return; }
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

<!-- One landmark around the whole workflow. The page has two root surfaces - the
     annotate tool and the floating thumbnail - and only one is visible at a time,
     so neither can be THE main on its own; without a wrapper axe reported no main
     landmark and the visible surface as content outside one. `display: contents`
     keeps both children exactly where they were in the layout. -->
<main class="page">

<!-- The annotate surface stays mounted (its canvas is set up on load); the phase
     only shows it once the user opens the capture from the floating thumbnail. -->
<div class="tool" class:hidden={phase !== "annotate" || captureFailure !== null}>
  {#if isSample}
    <!-- What is on the canvas is a drawing, not your screen. Said on the surface
         rather than left to be inferred, for the same reason the meetings list
         says "example meetings": a sample nobody labelled is indistinguishable
         from the real thing, and this one has a plausible account card with a
         plausible token in it. -->
    <p class="sample-note">{$t("s.sampleShot")}</p>
  {/if}
  {#if actionFailed}
    <!-- Above the stage rather than over it: the canvas is what the person is
         deciding about, and a refusal that covers the picture is its own
         problem. It stays until the next attempt settles it. -->
    <p class="action-failed" role="alert">
      {actionFailed === "save" ? $t("s.saveFailed") : $t("s.copyFailed")}
    </p>
  {/if}
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
          placeholder={$t("s.typePlaceholder")}
        ></textarea>
      {/if}
    </div>
  </div>

  <!-- The capture source. Only shown when there is a choice: on a machine with
       one screen and no toplevels there is nothing to pick between, and a
       control with one entry is furniture. -->
  {#if outputs.length + windows.length > 1}
    <div class="source">
      <label class="source-label" for="capture-source">{$t("s.source")}</label>
      <PopoverSelect
        value={source}
        options={[
          ...outputs.map((o) => ({
            value: `screen:${o.index}`,
            label: $t("s.source.screenSized", { name: o.name ?? String(o.index + 1), w: o.width, h: o.height }),
          })),
          ...windows.map((w) => ({
            value: `window:${w.index}`,
            label: w.title ? $t("s.source.window", { title: w.title }) : $t("s.source.windowUntitled"),
          })),
        ]}
        ariaLabel={$t("s.source")}
        onchange={(v) => void pickSource(v)}
      />
    </div>
  {/if}

  <div class="palette">
    <!-- The loop variable is not `t`: that is the translator store, and shadowing
         it here made `$t` resolve to the tool instead. -->
    {#each TOOLS as tl (tl.kind)}
      <Button variant={tool === tl.kind ? "secondary" : "ghost"} size="icon-sm" title={`${$t(tl.label)} (${tl.key})`} aria-label={$t(tl.label)} onclick={() => (tool = tl.kind)}>
        <tl.icon size={16} strokeWidth={1.75} />
      </Button>
    {/each}

    <span class="sep" aria-hidden="true"></span>

    <div class="swatches">
      {#each swatches as s (s.token)}
        <button class="swatch" class:active={color === s.hex} style={`background:${s.hex}`} aria-label={$t("s.swatch", { name: s.token.replace("--color-", "") })} onclick={() => (color = s.hex)}></button>
      {/each}
    </div>

    {#each SIZES as sz (sz.v)}
      <Button variant={size === sz.v ? "secondary" : "ghost"} size="icon-sm" title={$t(sz.label)} aria-label={$t(sz.label)} onclick={() => (size = sz.v)}>
        <span class="size-bar" style={`height:${sz.v}px`}></span>
      </Button>
    {/each}

    <span class="sep" aria-hidden="true"></span>

    <Button variant="ghost" size="icon-sm" title={$t("s.undoHint")} aria-label={$t("s.undo")} disabled={shapes.length === 0} onclick={undo}><Undo2 size={16} strokeWidth={1.75} /></Button>
    <Button variant="ghost" size="icon-sm" title={$t("s.redoHint")} aria-label={$t("s.redo")} disabled={redoStack.length === 0} onclick={redo}><Redo2 size={16} strokeWidth={1.75} /></Button>

    <span class="sep" aria-hidden="true"></span>

    <Button variant="outline" size="sm" title={$t("s.copyHint")} onclick={copy}><Copy size={15} strokeWidth={1.75} /> {$t("s.copy")}</Button>
    <Button variant="default" size="sm" title={$t("s.save")} onclick={save}><Download size={15} strokeWidth={1.75} /> {$t("s.save")}</Button>
  </div>
</div>

{#if captureFailure}
  <!-- The whole surface, because there is nothing to annotate and nothing to
       save. The tool above is hidden by its own phase check; this replaces the
       thumbnail that would otherwise carry a picture of nowhere. It names the
       cause: a compositor without the screencopy interface and a capture call
       that threw are different problems with different answers. -->
  <div class="no-capture" role="alert">
    <p class="no-capture-what">{$t("s.captureUnavailable")}</p>
    <p class="no-capture-why">{$t(WHY[captureFailure])}</p>
  </div>
{:else if phase === "thumbnail" && base}
  <FloatingThumbnail
    image={base}
    sample={isSample}
    onAnnotate={() => (phase = "annotate")}
    onCopy={() => base && copyCanvas(base)}
    onSave={() => base && saveCanvas(base)}
    onDismiss={autoSaveAndDismiss}
  />
{:else if phase === "dismissed"}
  <div class="dismissed">{$t("s.savedToPictures")}</div>
{/if}

</main>

<style>
  /* The landmark must not become a layout box: both surfaces stay direct
     children of the page as far as CSS is concerned. */
  .page {
    display: contents;
  }

  .tool.hidden {
    display: none;
  }
  .dismissed {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--text-base);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
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
  .action-failed {
    margin: 0 0 0.5rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--color-error, #f87171);
  }

  /* Above the canvas, in the warning colour rather than the error one: nothing
     went wrong, the picture is just not yours. */
  .source {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 0.5rem;
    font-size: 0.85rem;
  }
  .source-label {
    color: var(--color-fg-secondary, #9aa4b2);
  }

  .sample-note {
    margin: 0 0 0.5rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--color-warning, #fbbf24);
  }

  /* The whole window when there is no capture, because there is no picture to
     put beside it. */
  .no-capture {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    align-items: center;
    justify-content: center;
    height: 100vh;
    padding: 2rem;
    text-align: center;
  }

  .no-capture-what {
    margin: 0;
    max-width: 32rem;
    font-size: 0.95rem;
    font-weight: 500;
    color: var(--color-fg-primary, #e6e8ee);
  }

  .no-capture-why {
    margin: 0;
    max-width: 32rem;
    font-size: 0.85rem;
    color: var(--color-fg-secondary, #9aa4b2);
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
