<script lang="ts">
  /// The icon of one of our own apps: a plate and a glyph, drawn inline so it
  /// can move. An application or the shell sets `app` and `state`; this
  /// component owns the drawing and every transition. The freedesktop side
  /// (other apps, other shells, the Store's tile) keeps its raster set; this
  /// is the icon where the shell draws it itself: the launcher tile, the
  /// taskbar, a toast.
  ///
  /// The plate is three custom properties (`--app-icon-plate`,
  /// `--app-icon-line`, `--app-icon-glyph`, and the two hover variants)
  /// defaulting to the control chrome's percentages of the text colour the
  /// icon sits in, so a colour decision is a token, not a redraw. Motion is the Web
  /// Animations API on the glyph's parts; the duration is read from
  /// `--duration-normal`, which the shell's theme zeroes under reduce motion,
  /// so a zero means a jump and no gesture. Every animated part sets
  /// `transform-box: view-box` with its origin in grid units: a straight
  /// line has an empty fill-box, and WebKit ignores a CSS transform on a
  /// stroke marked non-scaling. The drawing is aria-hidden; the tile or
  /// toast that shows it carries the app's name.
  import { onDestroy } from "svelte";
  import { GLYPHS, type AppId, type AppIconState } from "./glyphs.js";

  let {
    app,
    state: current = "rest",
    size = 48,
    class: className,
  }: {
    /// Which app, by its desktop id without the prefix.
    app: AppId;
    /// The state, and the only thing a caller decides.
    state?: AppIconState;
    /// Rendered size in CSS pixels; the drawing scales.
    size?: number;
    class?: string;
  } = $props();

  let svg = $state<SVGSVGElement | null>(null);

  const markup = $derived(GLYPHS[app].join(""));

  // ---- motion primitives, each safe where the API is missing (jsdom) ----
  function reduced(): boolean {
    if (!svg) return true;
    const v = getComputedStyle(svg).getPropertyValue("--duration-normal").trim();
    if (v === "0ms" || v === "0s" || v === "0") return true;
    return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  }
  function animate(el: Element | null | undefined, frames: Keyframe[], options: KeyframeAnimationOptions): Animation | null {
    if (!el || typeof (el as HTMLElement).animate !== "function") return null;
    return (el as HTMLElement).animate(frames, options);
  }
  function origin(el: Element | undefined, x: number, y: number): void {
    if (el) (el as SVGElement).style.transformOrigin = `${x}px ${y}px`;
  }
  function parts(): SVGElement[] {
    return Array.from(svg?.querySelectorAll<SVGElement>(".glyph > *") ?? []);
  }
  function length(el: Element | undefined): number {
    const p = el as SVGGeometryElement | undefined;
    return typeof p?.getTotalLength === "function" ? p.getTotalLength() : 60;
  }

  // ---- hover: one gesture per glyph, the app's own act in one move ----
  // A gesture returns the animations it started. Those with `fill: forwards`
  // are held and played back when the hover ends; the rest are one pulse
  // and are simply cancelled.
  const held: KeyframeAnimationOptions = { duration: 260, easing: "cubic-bezier(0.4,0,0.2,1)", fill: "forwards" };
  const pulse: KeyframeAnimationOptions = { duration: 420, easing: "cubic-bezier(0.4,0,0.2,1)" };
  type Gesture = (p: SVGElement[]) => (Animation | null)[];
  const slide = (el: SVGElement | undefined, dx: number, dy = 0) =>
    animate(el, [{ transform: "translate(0,0)" }, { transform: `translate(${dx}px,${dy}px)` }], held);
  const twinkle = (els: (SVGElement | undefined)[], gap: number) =>
    els.map((el, i) => animate(el, [{ opacity: 1 }, { opacity: 0.3, offset: 0.4 }, { opacity: 1 }], { ...pulse, delay: i * gap }));
  const redraw = (el: SVGElement | undefined, ms: number, delay = 0) => {
    if (!el) return null;
    const len = length(el);
    el.style.strokeDasharray = `${len}`;
    const a = animate(el, [{ strokeDashoffset: len }, { strokeDashoffset: 0 }], { duration: ms, delay, easing: "cubic-bezier(0.2,0.8,0.2,1)" });
    a?.finished.then(() => (el.style.strokeDasharray = ""), () => (el.style.strokeDasharray = ""));
    return a;
  };
  const GESTURES: Record<AppId, Gesture> = {
    // the lid tilts up from the bottom left corner
    files: (p) => {
      origin(p[0], 2, 20);
      return [animate(p[0], [{ transform: "rotate(0)" }, { transform: "rotate(-5deg)" }], held)];
    },
    // each knob slides along its track, the track halves grow and shrink with it
    settings: (p) => {
      const rows: [number, number, number, number, number, number][] = [
        // knob, left track, right track, dx, left length, right length
        [2, 0, 6, 3, 7, 7],
        [7, 8, 4, 4, 5, 9],
        [3, 1, 5, -4, 9, 5],
      ];
      const out: (Animation | null)[] = [];
      for (const [k, l, r, dx, lw, rw] of rows) {
        const y = k === 2 ? 5 : k === 7 ? 12 : 19;
        origin(p[l], 3, y);
        origin(p[r], 21, y);
        out.push(slide(p[k], dx));
        out.push(animate(p[l], [{ transform: "scaleX(1)" }, { transform: `scaleX(${(lw + dx) / lw})` }], held));
        out.push(animate(p[r], [{ transform: "scaleX(1)" }, { transform: `scaleX(${(rw - dx) / rw})` }], held));
      }
      return out;
    },
    // the prompt steps forward and the line grows
    terminal: (p) => {
      origin(p[1], 11, 13);
      return [slide(p[0], 1.5), animate(p[1], [{ transform: "scaleX(1)" }, { transform: "scaleX(1.5)" }], held)];
    },
    // the trace runs once
    "system-monitor": (p) => [redraw(p[0], 360)],
    // the parcel is picked up: a small tilt about its centre
    store: (p) => {
      origin(p[0], 12, 12);
      return [animate(p[0], [{ transform: "rotate(0)" }, { transform: "rotate(-6deg)" }], held), ...[1, 2, 3].map((i) => {
        origin(p[i], 12, 12);
        return animate(p[i], [{ transform: "rotate(0)" }, { transform: "rotate(-6deg)" }], held);
      })];
    },
    // the flap folds down
    mail: (p) => {
      origin(p[0], 12, 7);
      return [animate(p[0], [{ transform: "scaleY(1)" }, { transform: "scaleY(0.55)" }], held)];
    },
    // the rings lift
    calendar: (p) => [slide(p[0], 0, -1), slide(p[1], 0, -1)],
    // the hand ticks
    clock: (p) => {
      origin(p[1], 12, 12);
      return [animate(p[1], [{ transform: "rotate(0)" }, { transform: "rotate(30deg)" }], held)];
    },
    // the capsule swells
    meetings: (p) => {
      origin(p[2], 12, 8.5);
      return [animate(p[2], [{ transform: "scale(1)" }, { transform: "scale(1.1)" }], held)];
    },
    // the pen tilts on its tip and the line grows
    "text-editor": (p) => {
      origin(p[1], 3, 21);
      origin(p[0], 13, 21);
      return [
        animate(p[1], [{ transform: "rotate(0)" }, { transform: "rotate(-8deg)" }], held),
        animate(p[0], [{ transform: "scaleX(1)" }, { transform: "scaleX(1.2)" }], held),
      ];
    },
    // the sun rises
    viewers: (p) => [slide(p[1], 0, -1.2)],
    // the lines step in, one after the other
    pdf: (p) => [2, 3, 4].map((i, n) => animate(p[i], [{ transform: "translate(0,0)" }, { transform: "translate(1px,0)" }], { ...held, delay: n * 40 })),
    // the shutter blinks
    screenshot: (p) => {
      origin(p[1], 12, 13);
      return [animate(p[1], [{ transform: "scale(1)" }, { transform: "scale(0.72)", offset: 0.4 }, { transform: "scale(1)" }], { duration: 320, easing: "cubic-bezier(0.4,0,0.2,1)" })];
    },
    // the paths flicker round the graph
    knowledge: (p) => twinkle([p[2], p[3], p[4], p[5], p[6], p[7], p[1]], 35),
    // the spark pulses and the small stars blink
    harness: (p) => {
      origin(p[0], 12, 12);
      return [
        animate(p[0], [{ transform: "scale(1)" }, { transform: "scale(0.94)", offset: 0.4 }, { transform: "scale(1.05)" }], { duration: 320, easing: "cubic-bezier(0.4,0,0.2,1)", fill: "forwards" }),
        ...twinkle([p[1], p[2], p[3]], 60),
      ];
    },
    // the glass moves over the folder
    "file-picker": (p) => [slide(p[1], -1.5, -1.5), slide(p[2], -1.5, -1.5)],
  };

  // ---- the current machine ----
  let hovering: Animation[] = [];
  let looping: Animation | null = null;
  let shown: AppIconState | null = null;

  function endHover(): void {
    for (const a of hovering) {
      if (a.effect?.getTiming().fill === "forwards") {
        a.reverse();
        a.finished.then(() => a.cancel(), () => {});
      } else a.cancel();
    }
    hovering = [];
  }
  function stopLoop(): void {
    looping?.cancel();
    looping = null;
  }
  function apply(next: AppIconState): void {
    if (!svg) return;
    if (shown === "hover" && next !== "hover") endHover();
    stopLoop();
    shown = next;
    if (reduced() || next === "rest") return;
    const glyph = svg.querySelector<SVGGElement>(".glyph");
    if (next === "hover") {
      hovering = GESTURES[app](parts()).filter((a): a is Animation => a !== null);
    } else if (next === "open") {
      animate(svg, [{ transform: "scale(1)" }, { transform: "scale(0.92)", offset: 0.3 }, { transform: "scale(1.02)", offset: 0.7 }, { transform: "scale(1)" }], { duration: 320, easing: "cubic-bezier(0.2,0.8,0.2,1)" });
      for (const el of parts()) redraw(el, 240, 60);
    } else if (next === "notify") {
      animate(glyph, [{ transform: "rotate(0)" }, { transform: "rotate(-3deg)", offset: 0.25 }, { transform: "rotate(3deg)", offset: 0.6 }, { transform: "rotate(0)" }], { duration: 360, easing: "cubic-bezier(0.4,0,0.2,1)" });
    } else if (next === "busy") {
      looping = animate(glyph, [{ opacity: 1 }, { opacity: 0.35 }, { opacity: 1 }], { duration: 1200, iterations: Infinity, easing: "cubic-bezier(0.4,0,0.2,1)" });
    }
  }
  $effect(() => {
    const next = current;
    if (!svg) return;
    if (shown !== next) apply(next);
  });
  onDestroy(() => {
    stopLoop();
    for (const a of hovering) a.cancel();
  });
</script>

<span class="app-icon {className ?? ''}" style="--app-icon-size: {size}px" data-app={app} data-state={current}>
  <svg bind:this={svg} viewBox="0 0 40 40" aria-hidden="true" focusable="false">
    <rect class="plate" x="0.5" y="0.5" width="39" height="39" rx="9" />
    <g transform="translate(8 8)">
      <g class="glyph" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{@html markup}</g>
    </g>
  </svg>
</span>

<style>
  .app-icon {
    display: inline-block;
    width: var(--app-icon-size);
    height: var(--app-icon-size);
    line-height: 0;
    color: currentColor;
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: visible;
    transform-box: view-box;
    transform-origin: 20px 20px;
  }
  .plate {
    fill: var(--app-icon-plate, color-mix(in srgb, currentColor 8%, transparent));
    stroke: var(--app-icon-line, color-mix(in srgb, currentColor 12%, transparent));
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
    transition:
      fill var(--duration-fast, 150ms) var(--ease-out, ease-out),
      stroke var(--duration-fast, 150ms) var(--ease-out, ease-out);
  }
  [data-state="hover"] .plate {
    fill: var(--app-icon-plate-hover, color-mix(in srgb, currentColor 13%, transparent));
    stroke: var(--app-icon-line-hover, color-mix(in srgb, currentColor 22%, transparent));
  }
  .glyph {
    stroke: var(--app-icon-glyph, currentColor);
    transform-box: view-box;
    transform-origin: 12px 12px;
  }
  .glyph > :global(*) {
    transform-box: view-box;
  }
</style>
