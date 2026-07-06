/// The annotation model: a retained list of shapes drawn over the captured image
/// on a single canvas (redrawn on every change, so the canvas is always the
/// export-ready composite). Satty/Flameshot model - annotate directly on the
/// image, no separate editor document.

/// A tool the palette offers.
export type ToolKind =
  | "select"
  | "crop"
  | "arrow"
  | "box"
  | "ellipse"
  | "text"
  | "pen"
  | "highlight"
  | "blur"
  | "number";

/// The drawable shape kinds (everything except the modal select/crop tools).
export type ShapeKind = Exclude<ToolKind, "select" | "crop">;

export interface Point {
  x: number;
  y: number;
}

/// One placed annotation.
export interface Shape {
  id: number;
  kind: ShapeKind;
  color: string;
  /// Stroke width / font scale / pixelation coarseness, by kind.
  size: number;
  start: Point;
  end: Point;
  /// Freehand + highlighter path.
  points?: Point[];
  /// Text content (kind === "text").
  text?: string;
  /// Step number (kind === "number").
  n?: number;
}

/// A rectangle from two corners, normalized to top-left + size.
export function rectOf(a: Point, b: Point): { x: number; y: number; w: number; h: number } {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.abs(a.x - b.x),
    h: Math.abs(a.y - b.y),
  };
}

/// Draw one shape onto the context. `base` (the untouched image bitmap) is needed
/// only by the blur tool, which pixelates the region beneath it.
export function drawShape(ctx: CanvasRenderingContext2D, s: Shape, base?: CanvasImageSource): void {
  ctx.save();
  ctx.strokeStyle = s.color;
  ctx.fillStyle = s.color;
  ctx.lineWidth = s.size;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  switch (s.kind) {
    case "box": {
      const r = rectOf(s.start, s.end);
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      break;
    }
    case "ellipse": {
      const r = rectOf(s.start, s.end);
      ctx.beginPath();
      ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, r.w / 2, r.h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    }
    case "arrow":
      drawArrow(ctx, s.start, s.end, s.size);
      break;
    case "pen":
    case "highlight": {
      const pts = s.points ?? [s.start, s.end];
      if (s.kind === "highlight") {
        ctx.globalAlpha = 0.35;
        ctx.lineWidth = s.size * 3;
      }
      ctx.beginPath();
      ctx.moveTo(pts[0].x, pts[0].y);
      for (const p of pts.slice(1)) ctx.lineTo(p.x, p.y);
      ctx.stroke();
      break;
    }
    case "text": {
      const px = 12 + s.size * 4;
      ctx.font = `600 ${px}px "Inter Variable", system-ui, sans-serif`;
      ctx.textBaseline = "top";
      for (const [i, line] of (s.text ?? "").split("\n").entries()) {
        ctx.fillText(line, s.start.x, s.start.y + i * px * 1.25);
      }
      break;
    }
    case "number":
      drawNumber(ctx, s);
      break;
    case "blur":
      if (base) drawBlur(ctx, s, base);
      break;
  }
  ctx.restore();
}

function drawArrow(ctx: CanvasRenderingContext2D, a: Point, b: Point, size: number): void {
  const head = 8 + size * 2.5;
  const ang = Math.atan2(b.y - a.y, b.x - a.x);
  ctx.beginPath();
  ctx.moveTo(a.x, a.y);
  ctx.lineTo(b.x, b.y);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(b.x, b.y);
  ctx.lineTo(b.x - head * Math.cos(ang - Math.PI / 7), b.y - head * Math.sin(ang - Math.PI / 7));
  ctx.lineTo(b.x - head * Math.cos(ang + Math.PI / 7), b.y - head * Math.sin(ang + Math.PI / 7));
  ctx.closePath();
  ctx.fill();
}

function drawNumber(ctx: CanvasRenderingContext2D, s: Shape): void {
  const r = 11 + s.size * 3;
  ctx.beginPath();
  ctx.arc(s.start.x, s.start.y, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#ffffff";
  ctx.font = `700 ${r}px "Inter Variable", system-ui, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(String(s.n ?? 1), s.start.x, s.start.y + 1);
}

// Redaction: pixelate the region under the shape by downscaling then upscaling a
// slice of the untouched base image. Coarseness scales with the tool size.
function drawBlur(ctx: CanvasRenderingContext2D, s: Shape, base: CanvasImageSource): void {
  const r = rectOf(s.start, s.end);
  if (r.w < 2 || r.h < 2) return;
  const block = Math.max(4, s.size * 4);
  const sw = Math.max(1, Math.round(r.w / block));
  const sh = Math.max(1, Math.round(r.h / block));
  const tmp = document.createElement("canvas");
  tmp.width = sw;
  tmp.height = sh;
  const tctx = tmp.getContext("2d");
  if (!tctx) return;
  tctx.drawImage(base, r.x, r.y, r.w, r.h, 0, 0, sw, sh);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(tmp, 0, 0, sw, sh, r.x, r.y, r.w, r.h);
  ctx.imageSmoothingEnabled = true;
}
