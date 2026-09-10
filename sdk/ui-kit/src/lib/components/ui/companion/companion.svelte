<script lang="ts">
  /// The companion: one drawing, ten states, and the state name is the whole
  /// interface. An application sets `state`; this component owns every
  /// transition. It appears on three surfaces only (spec, section 2): beside
  /// the assistant's text box, at the launcher's input in AI mode, and in
  /// first-run setup. Nowhere else.
  ///
  /// Motion: the six morphable parts are SMIL `<animate>` on `d` (CSS `d` does
  /// not animate on WebKitGTK), the head moves by a WAAPI transform, the
  /// accents pop by opacity and scale. Reduce motion is the person's choice in
  /// appearance.toml: the shell's theme zeroes `--duration-normal` and the kit
  /// injects it into every app, so that variable is read here rather than the
  /// media query the engine may never propagate (spec, section 6). Under
  /// reduce motion a state change is a jump, and there are no gestures and no
  /// accents. `thinking` is the only looping state and stops itself after
  /// five seconds. The drawing is aria-hidden; a status region beside it says
  /// what happened in words.
  import { onDestroy } from "svelte";
  import { kt } from "../../../i18n/messages.kit";
  import {
    FIXED,
    EYE,
    MOUTH,
    PAW,
    POSES,
    SLOTS,
    GESTURES,
    IDLE_STATES,
    IDLE_GAP_MIN,
    IDLE_GAP_MAX,
    TALK_SHAPES,
    THINKING_LOOP_MS,
    VIEW_BUST,
    VIEW_FIGURE,
    type CompanionState,
    type Gesture,
    type Slot,
  } from "./rig.js";

  let {
    state: current = "resting",
    size = 40,
    bust = false,
    radius,
    talking = false,
    idle = true,
    class: className,
  }: {
    /// The state, and the only thing a caller decides.
    state?: CompanionState;
    /// Rendered size in CSS pixels; the drawing scales.
    size?: number;
    /// Head and shoulders in a frame, like a profile picture. The frame's
    /// corner radius follows the card token unless `radius` says otherwise.
    bust?: boolean;
    /// A CSS length or `50%` for a full circle; only read when `bust` is set.
    radius?: string;
    /// While an answer streams: the mouth moves. Only read in `answering`.
    talking?: boolean;
    /// Rare gestures on a timer in the quiet states. Off under reduce motion.
    idle?: boolean;
    class?: string;
  } = $props();

  let svg = $state<SVGSVGElement | null>(null);

  const initial = POSES.resting;

  // ---- motion primitives, each safe where the API is missing (jsdom) ----
  function reduced(): boolean {
    if (!svg) return true;
    const v = getComputedStyle(svg).getPropertyValue("--duration-normal").trim();
    if (v === "0ms" || v === "0s" || v === "0") return true;
    return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  }
  function duration(base: number): number {
    return reduced() ? 0 : base;
  }
  function part(slot: Slot | string): SVGPathElement | null {
    return svg?.querySelector<SVGPathElement>(`.${slot}`) ?? null;
  }
  /// Morph one slot's `d` from where it is to `to`. The base attribute is set
  /// at once so the next morph starts from the truth; SMIL draws the way there.
  function morph(slot: Slot, to: string, ms = 250): void {
    const p = part(slot);
    if (!p) return;
    const from = p.getAttribute("d");
    if (from === to) return;
    p.setAttribute("d", to);
    const a = p.querySelector("animate") as (SVGAnimateElement & { beginElement?: () => void }) | null;
    const d = duration(ms);
    if (!a || d === 0 || typeof a.beginElement !== "function") return;
    a.setAttribute("from", from ?? to);
    a.setAttribute("to", to);
    a.setAttribute("dur", `${d}ms`);
    a.beginElement();
  }
  function animate(el: Element | null, frames: Keyframe[], options: KeyframeAnimationOptions): Animation | null {
    if (!el || typeof (el as HTMLElement).animate !== "function") return null;
    return (el as HTMLElement).animate(frames, options);
  }
  function currentTransform(el: Element | null): string {
    if (!el) return "none";
    const t = getComputedStyle(el).transform;
    return t && t !== "none" ? t : "none";
  }
  function setHead(tf: string): void {
    const head = svg?.querySelector(".head") ?? null;
    if (!head) return;
    const d = duration(250);
    if (d === 0) {
      (head as SVGGElement).style.transform = tf;
      return;
    }
    animate(head, [{ transform: currentTransform(head) }, { transform: tf }], { duration: d, easing: "cubic-bezier(0.2,0.8,0.2,1)", fill: "forwards" });
  }
  function setOvals(which: string): void {
    for (const [cls, on] of [["ovL", which.includes("L")], ["ovR", which.includes("R")]] as const) {
      const el = svg?.querySelector(`.${cls}`) as SVGPathElement | null;
      if (!el) continue;
      const d = duration(150);
      if (d === 0) {
        el.style.opacity = on ? "1" : "0";
        continue;
      }
      animate(el, [{ opacity: getComputedStyle(el).opacity }, { opacity: on ? 1 : 0 }], { duration: d, delay: on ? 120 : 0, fill: "forwards" });
    }
  }

  // ---- accents: one coloured object, once ----
  let thinkingTimer: ReturnType<typeof setTimeout> | null = null;
  function clearAccents(): void {
    if (thinkingTimer) clearTimeout(thinkingTimer);
    thinkingTimer = null;
    svg?.querySelectorAll<SVGGElement>(".accent").forEach((el) => {
      el.getAnimations?.().forEach((a) => a.cancel());
      el.querySelectorAll("circle").forEach((c) => c.getAnimations?.().forEach((a) => a.cancel()));
      el.style.opacity = "0";
    });
  }
  function accent(kind: string | undefined): void {
    clearAccents();
    if (!kind || reduced()) return;
    const el = svg?.querySelector<SVGGElement>(`.accent.${kind}`) ?? null;
    if (!el) return;
    const pop = "cubic-bezier(0.34,1.56,0.64,1)";
    if (kind === "heart") {
      animate(el, [{ opacity: 0, transform: "scale(0)" }, { opacity: 1, transform: "scale(1)", offset: 0.35 }, { opacity: 1, transform: "scale(1)", offset: 0.8 }, { opacity: 0, transform: "scale(.6) translateY(-3px)" }], { duration: 1400, easing: pop, delay: 150 });
    } else if (kind === "tear") {
      animate(el, [{ opacity: 0, transform: "translateY(0) scale(.6)" }, { opacity: 1, transform: "translateY(1px) scale(1)", offset: 0.25 }, { opacity: 1, transform: "translateY(7px) scale(1)", offset: 0.8 }, { opacity: 0, transform: "translateY(9px) scale(.8)" }], { duration: 1600, easing: "cubic-bezier(0.4,0,0.2,1)", delay: 200 });
    } else if (kind === "question") {
      animate(el, [{ opacity: 0, transform: "scale(0) rotate(-12deg)" }, { opacity: 1, transform: "scale(1) rotate(0)", offset: 0.4 }, { opacity: 1, offset: 0.85 }, { opacity: 0 }], { duration: 1600, easing: pop, delay: 150 });
    } else if (kind === "dots") {
      el.style.opacity = "1";
      const dots = el.querySelectorAll("circle");
      dots.forEach((c, i) => animate(c, [{ opacity: 0.2 }, { opacity: 1, offset: 0.3 }, { opacity: 0.2, offset: 0.6 }, { opacity: 0.2 }], { duration: 900, delay: i * 160, iterations: Infinity }));
      thinkingTimer = setTimeout(() => {
        dots.forEach((c) => c.getAnimations?.().forEach((a) => a.cancel()));
        animate(el, [{ opacity: 1 }, { opacity: 0 }], { duration: 200, fill: "forwards" });
      }, THINKING_LOOP_MS);
    }
  }

  // ---- talking: the mouth cycles while the answer streams ----
  let talkTimer: ReturnType<typeof setTimeout> | null = null;
  let bob: Animation | null = null;
  function stopTalking(): void {
    if (talkTimer) clearTimeout(talkTimer);
    talkTimer = null;
    bob?.cancel();
    bob = null;
    if (current === "answering") morph("mouth", POSES.answering.mouth, 120);
  }
  function startTalking(): void {
    if (talkTimer || reduced()) return;
    bob = animate(svg?.querySelector(".head") ?? null, [{ transform: "translateY(0)" }, { transform: "translateY(-.5px)" }], { duration: 170, direction: "alternate", iterations: Infinity });
    let i = 0;
    const step = () => {
      if (current !== "answering" || !talking) {
        stopTalking();
        return;
      }
      morph("mouth", TALK_SHAPES[i++ % TALK_SHAPES.length], 80);
      talkTimer = setTimeout(step, 90 + Math.random() * 90);
    };
    step();
  }

  // ---- idle gestures ----
  let idleTimer: ReturnType<typeof setTimeout> | null = null;
  const gestureTimers: ReturnType<typeof setTimeout>[] = [];
  function later(fn: () => void, ms: number): void {
    gestureTimers.push(setTimeout(fn, ms));
  }
  function gesture(kind: Gesture): void {
    if (!svg || reduced() || !IDLE_STATES.includes(current)) return;
    const pose = POSES[current];
    const head = svg.querySelector(".head");
    const headTf = pose.head;
    if (kind === "blink" && pose.eyeL === EYE.arcL) {
      morph("eyeL", EYE.flatL, 70);
      morph("eyeR", EYE.flatR, 70);
      later(() => {
        morph("eyeL", pose.eyeL, 110);
        morph("eyeR", pose.eyeR, 110);
      }, 120);
    } else if (kind === "ear") {
      animate(svg.querySelector(".earR"), [{ transform: "rotate(0)" }, { transform: "rotate(14deg)", offset: 0.3 }, { transform: "rotate(-6deg)", offset: 0.65 }, { transform: "rotate(0)" }], { duration: 420, easing: "ease-out" });
    } else if (kind === "stretch") {
      animate(head, [{ transform: headTf }, { transform: "translateY(-3px)", offset: 0.3 }, { transform: "translateY(-3px)", offset: 0.7 }, { transform: headTf }], { duration: 1200, easing: "cubic-bezier(0.4,0,0.2,1)" });
      morph("pawL", PAW.upL, 300);
      morph("pawR", PAW.upR, 300);
      later(() => {
        morph("pawL", pose.pawL, 400);
        morph("pawR", pose.pawR, 400);
      }, 800);
    } else if (kind === "scratch") {
      const ov = svg.querySelector(".ovR") as SVGPathElement | null;
      morph("pawR", PAW.scratchR, 260);
      animate(head, [{ transform: headTf }, { transform: "rotate(-5deg)", offset: 0.2 }, { transform: "rotate(-5deg)", offset: 0.8 }, { transform: headTf }], { duration: 1500, easing: "ease-in-out" });
      animate(ov, [{ opacity: 0 }, { opacity: 1 }], { duration: 200, delay: 120, fill: "forwards" });
      later(() => {
        for (const el of [part("pawR"), ov]) animate(el, [{ transform: "translateY(0)" }, { transform: "translateY(-1.3px)" }], { duration: 110, direction: "alternate", iterations: 6 });
      }, 260);
      later(() => {
        morph("pawR", pose.pawR, 300);
        animate(ov, [{ opacity: 1 }, { opacity: pose.ovals.includes("R") ? 1 : 0 }], { duration: 150, fill: "forwards" });
      }, 1100);
    } else if (kind === "yawn") {
      morph("mouth", MOUTH.yawn, 350);
      morph("eyeL", EYE.flatL, 350);
      morph("eyeR", EYE.flatR, 350);
      animate(head, [{ transform: headTf }, { transform: "translateY(-1.5px) rotate(3deg)", offset: 0.3 }, { transform: "translateY(-1.5px) rotate(3deg)", offset: 0.7 }, { transform: headTf }], { duration: 1600, easing: "ease-in-out" });
      later(() => {
        morph("mouth", pose.mouth, 400);
        morph("eyeL", pose.eyeL, 400);
        morph("eyeR", pose.eyeR, 400);
      }, 1100);
    }
  }
  function scheduleIdle(): void {
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = null;
    if (!idle || !IDLE_STATES.includes(current)) return;
    const gap = IDLE_GAP_MIN + Math.random() * (IDLE_GAP_MAX - IDLE_GAP_MIN);
    idleTimer = setTimeout(() => {
      if (typeof document === "undefined" || document.visibilityState === "visible") {
        gesture(GESTURES[Math.floor(Math.random() * GESTURES.length)]);
      }
      scheduleIdle();
    }, gap);
  }
  function cancelGestures(): void {
    gestureTimers.splice(0).forEach(clearTimeout);
  }

  // ---- the current machine ----
  let shown: CompanionState | null = null;
  function apply(next: CompanionState): void {
    if (!svg) return;
    cancelGestures();
    stopTalking();
    const pose = POSES[next];
    for (const slot of SLOTS) morph(slot, pose[slot]);
    setHead(pose.head);
    setOvals(pose.ovals);
    accent(pose.accent);
    shown = next;
  }
  $effect(() => {
    const next = current;
    if (!svg) return;
    if (shown !== next) apply(next);
    scheduleIdle();
  });
  $effect(() => {
    const on = talking && current === "answering";
    if (!svg) return;
    if (on) startTalking();
    else stopTalking();
  });
  onDestroy(() => {
    cancelGestures();
    if (idleTimer) clearTimeout(idleTimer);
    if (talkTimer) clearTimeout(talkTimer);
    if (thinkingTimer) clearTimeout(thinkingTimer);
    bob?.cancel();
  });

  const frame = $derived(bust ? `border-radius: ${radius ?? "var(--radius-card)"}` : "");
</script>

<span class="companion {bust ? 'bust' : ''} {className ?? ''}" style="--companion-size: {size}px; {frame}">
  <svg bind:this={svg} viewBox={bust ? VIEW_BUST : VIEW_FIGURE} aria-hidden="true" focusable="false">
    <defs>
      <mask id="companion-tail-mask">
        <rect x="-64" y="-64" width="192" height="192" fill="white" />
        <path d="{FIXED.body}Z" fill="black" />
      </mask>
    </defs>
    <g fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
      <path class="tail" d={initial.tail} mask="url(#companion-tail-mask)"><animate attributeName="d" begin="indefinite" dur="250ms" fill="freeze" calcMode="spline" keySplines="0.2 0.8 0.2 1" keyTimes="0;1" /></path>
      <path d={FIXED.body} />
      <path class="pawL" d={initial.pawL}><animate attributeName="d" begin="indefinite" dur="250ms" fill="freeze" calcMode="spline" keySplines="0.2 0.8 0.2 1" keyTimes="0;1" /></path>
      <path class="pawR" d={initial.pawR}><animate attributeName="d" begin="indefinite" dur="250ms" fill="freeze" calcMode="spline" keySplines="0.2 0.8 0.2 1" keyTimes="0;1" /></path>
      <path class="oval ovL" d={FIXED.pawOvalL} />
      <path class="oval ovR" d={FIXED.pawOvalR} />
      <path d={FIXED.feet} />
      <g class="head">
        <path d={FIXED.head} />
        <path d={FIXED.earL} />
        <path class="earR" d={FIXED.earR} />
        <path d={FIXED.whiskers} />
        <path d={FIXED.lip} />
        <path class="mouth" d={initial.mouth}><animate attributeName="d" begin="indefinite" dur="250ms" fill="freeze" calcMode="spline" keySplines="0.2 0.8 0.2 1" keyTimes="0;1" /></path>
        <path class="eyeL" d={initial.eyeL} stroke-width="2.6"><animate attributeName="d" begin="indefinite" dur="250ms" fill="freeze" calcMode="spline" keySplines="0.2 0.8 0.2 1" keyTimes="0;1" /></path>
        <path class="eyeR" d={initial.eyeR} stroke-width="2.6"><animate attributeName="d" begin="indefinite" dur="250ms" fill="freeze" calcMode="spline" keySplines="0.2 0.8 0.2 1" keyTimes="0;1" /></path>
        <path d={FIXED.nose} stroke-width="3.4" />
      </g>
      <g class="accent dots" fill="currentColor" stroke="none"><circle cx="24" cy="4.5" r="1.6" /><circle cx="32" cy="2.6" r="1.6" /><circle cx="40" cy="4.5" r="1.6" /></g>
      <g transform="translate(51 13)"><g class="accent heart"><path d={FIXED.heart} class="error" stroke="none" /></g></g>
      <g transform="translate(24.5 31)"><g class="accent tear"><path d={FIXED.tear} class="info" stroke="none" /></g></g>
      <g transform="translate(50 9)"><g class="accent question"><path d={FIXED.question} class="info-stroke" fill="none" stroke-width="2.2" /><circle cx="1.4" cy="4.8" r="1.3" class="info" stroke="none" /></g></g>
    </g>
  </svg>
  <span class="status" role="status">{$kt(`k.companion.${current}`)}</span>
</span>

<style>
  .companion {
    display: inline-block;
    width: var(--companion-size);
    height: var(--companion-size);
    line-height: 0;
    color: currentColor;
  }
  .companion.bust {
    overflow: hidden;
    box-sizing: border-box;
    border: 1.5px solid currentColor;
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .bust svg {
    overflow: hidden;
  }
  .head {
    transform-box: view-box;
    transform-origin: 32px 34px;
  }
  .earR {
    transform-box: view-box;
    transform-origin: 44px 15.5px;
  }
  .accent {
    opacity: 0;
    transform-box: fill-box;
    transform-origin: center;
  }
  .oval {
    opacity: 0;
  }
  .error {
    fill: var(--color-error, #e5484d);
  }
  .info {
    fill: var(--color-info, #3b82f6);
  }
  .info-stroke {
    stroke: var(--color-info, #3b82f6);
  }
  /* the textual twin of the drawing: read by a screen reader, never seen */
  .status {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
