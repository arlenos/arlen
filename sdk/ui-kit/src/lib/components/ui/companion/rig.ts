// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// The companion's drawing, as data. Every part that changes between states
/// keeps one fixed command skeleton (one cubic per eye and paw, two per
/// mouth, three for the tail) so SMIL can interpolate `d` between any two
/// states; CSS `d` does not animate on WebKitGTK, which is why the morphs are
/// SMIL and the head moves by transform. The figure is a 64 unit grid, stroke
/// 2.4, `currentColor`, no fill. The species is never named here or anywhere.

/// The ten states of the companion spec, section 3. The name is the whole
/// interface between an application and the drawing.
export type CompanionState =
  | "resting"
  | "attentive"
  | "listening"
  | "thinking"
  | "answering"
  | "unsure"
  | "refused"
  | "failed"
  | "greeting"
  | "done";

/// The states, in the spec's order.
export const COMPANION_STATES: readonly CompanionState[] = [
  "resting",
  "attentive",
  "listening",
  "thinking",
  "answering",
  "unsure",
  "refused",
  "failed",
  "greeting",
  "done",
];

/// The parts that never change.
export const FIXED = {
  head: "M32 8c-9.5 0-16 6-17.5 13-.5 3 0 6.5 1 9 2.5 5 8.5 7 16.5 7s14-2 16.5-7c1-2.5 1.5-6 1-9C48 14 41.5 8 32 8Z",
  earL: "M15.5 15.5a3.4 3.4 0 0 1 4.5-4.5",
  earR: "M48.5 15.5a3.4 3.4 0 0 0-4.5-4.5",
  whiskers:
    "M15.5 26.5c2.5 0 4.5.4 6.5 1M16 30c2.3-.5 4.2-.5 6-.2M48.5 26.5c-2.5 0-4.5.4-6.5 1M48 30c-2.3-.5-4.2-.5-6-.2",
  nose: "M29.8 25.2h4.4",
  lip: "M32 27v1.2",
  body: "M21 34c-3 5-4 10-4 15 0 5.5 6.5 8 15 8s15-2.5 15-8c0-5-1-10-4-15",
  feet: "M20.5 57.5a3.8 2.2 0 1 0 7.6 0a3.8 2.2 0 1 0-7.6 0M35.9 57.5a3.8 2.2 0 1 0 7.6 0a3.8 2.2 0 1 0-7.6 0",
  pawOvalL: "M8 31a2.6 3.3 0 1 0 5.2 0a2.6 3.3 0 1 0-5.2 0",
  pawOvalR: "M50.8 31a2.6 3.3 0 1 0 5.2 0a2.6 3.3 0 1 0-5.2 0",
  heart: "M0 3c-3.2-3.2-6.6-.8-5.4 2 .9 2.4 3.4 4 5.4 5.6 2-1.6 4.5-3.2 5.4-5.6 1.2-2.8-2.2-5.2-5.4-2Z",
  tear: "M0 0c-1.6 2.6-2.4 4-2.4 5.3a2.4 2.4 0 0 0 4.8 0C2.4 4 1.6 2.6 0 0Z",
  question: "M-2.6-2.4c0-2.2 1.4-3.6 3.4-3.6s3.2 1.3 3.2 3c0 2.2-2.6 2.6-2.6 4.8",
} as const;

/// The viewBox of the whole figure, and of the bust: head and shoulders, the
/// paws folded at the rim, the frame supplied by the container's radius.
export const VIEW_FIGURE = "0 0 64 64";
export const VIEW_BUST = "6.4 -2 51.2 51.2";

export const EYE = {
  arcL: "M20.4 20.6c1.1-1.8 4.3-1.8 5.4 0",
  arcR: "M38.2 20.6c1.1-1.8 4.3-1.8 5.4 0",
  upL: "M20.4 19.1c1.1-1.8 4.3-1.8 5.4 0",
  upR: "M38.2 19.1c1.1-1.8 4.3-1.8 5.4 0",
  sadL: "M20.4 19.2c1.1 1.8 4.3 1.8 5.4 0",
  sadR: "M38.2 19.2c1.1 1.8 4.3 1.8 5.4 0",
  flatL: "M20.4 20.2c1.1 0 4.3 0 5.4 0",
  flatR: "M38.2 20.2c1.1 0 4.3 0 5.4 0",
} as const;

export const MOUTH = {
  shut: "M29 28.9c1.2 1.2 2.2.9 3-.7.8 1.6 1.8 1.9 3 .7",
  open: "M30.2 29.8c0-2.1 3.6-2.1 3.6 0 0 2.1-3.6 2.1-3.6 0",
  half: "M30.2 29.6c0-1.1 3.6-1.1 3.6 0 0 1.1-3.6 1.1-3.6 0",
  small: "M30.6 29.4c.4-.5 2.8-.5 3.2 0-.4.5-2.8.5-3.2 0",
  yawn: "M29.4 29.8c0-3.2 5.2-3.2 5.2 0 0 3.2-5.2 3.2-5.2 0",
  flat: "M29.5 29.4c.8 0 1.7 0 2.5 0 .8 0 1.7 0 2.5 0",
  wavy: "M28.8 29.6c1-1.1 2.1-1.1 3.2 0 1.1 1.1 2.2 1.1 3.2 0",
  frown: "M29.2 30.4c.6-.9 1.6-1.6 2.8-1.6 1.2 0 2.2.7 2.8 1.6",
  big: "M28.4 28.8c.8 1.3 2 2 3.6 2 1.6 0 2.8-.7 3.6-2",
} as const;

export const PAW = {
  foldL: "M26 42.5c1.2-2.4 4-2.4 6-.8",
  foldR: "M32 41.7c2-1.6 4.8-1.6 6 .8",
  chinR: "M38 36c1-1.8 3.4-2 5-.5",
  outL: "M17.5 43c-3 .3-5.2 2.2-5.6 5",
  outR: "M46.5 43c3 .3 5.2 2.2 5.6 5",
  stopL: "M17.5 42.5c-3-1-4.6-3.4-4.2-6.4",
  stopR: "M46.5 42.5c3-1 4.6-3.4 4.2-6.4",
  upL: "M18 40c-2.5-1.5-4.5-3.5-5.5-6.5",
  upR: "M46 40c2.5-1.5 4.5-3.5 5.5-6.5",
  scratchR: "M46.5 42.5c4.5-2.5 6.5-7 6-11.5",
} as const;

export const TAIL = {
  curl: "M40 55c9-1.5 13.5-6.5 12-13.5-.7-3.4-4.7-3.6-6.2-.6-1.5 3-3 8-9 10.5",
  down: "M40 48c6-1 13.5 1.5 13.5 6 0 2.2-1.2 3.8-3 4.6-2.2 1-5 1.2-8 .9",
} as const;

/// The morphable slots, in the order the component keeps them.
export const SLOTS = ["eyeL", "eyeR", "mouth", "pawL", "pawR", "tail"] as const;
export type Slot = (typeof SLOTS)[number];

/// A coloured object a state may pop once; the figure itself stays one colour.
export type Accent = "dots" | "heart" | "tear" | "question";

/// One state's pose: the six slots, the head transform, which raised-paw
/// ovals show, and the accent if any.
export interface Pose {
  eyeL: string;
  eyeR: string;
  mouth: string;
  pawL: string;
  pawR: string;
  tail: string;
  /// A CSS transform for the head group, `none` for the resting position.
  head: string;
  ovals: "" | "L" | "R" | "LR";
  accent?: Accent;
}

export const POSES: Record<CompanionState, Pose> = {
  resting: { eyeL: EYE.arcL, eyeR: EYE.arcR, mouth: MOUTH.shut, pawL: PAW.foldL, pawR: PAW.foldR, tail: TAIL.curl, head: "none", ovals: "" },
  attentive: { eyeL: EYE.arcL, eyeR: EYE.arcR, mouth: MOUTH.shut, pawL: PAW.foldL, pawR: PAW.foldR, tail: TAIL.curl, head: "translateY(-1.6px)", ovals: "" },
  listening: { eyeL: EYE.arcL, eyeR: EYE.arcR, mouth: MOUTH.shut, pawL: PAW.foldL, pawR: PAW.foldR, tail: TAIL.curl, head: "rotate(-7deg)", ovals: "" },
  thinking: { eyeL: EYE.upL, eyeR: EYE.upR, mouth: MOUTH.shut, pawL: PAW.foldL, pawR: PAW.chinR, tail: TAIL.curl, head: "none", ovals: "", accent: "dots" },
  answering: { eyeL: EYE.arcL, eyeR: EYE.arcR, mouth: MOUTH.open, pawL: PAW.outL, pawR: PAW.outR, tail: TAIL.curl, head: "none", ovals: "" },
  unsure: { eyeL: EYE.flatL, eyeR: EYE.arcR, mouth: MOUTH.wavy, pawL: PAW.foldL, pawR: PAW.foldR, tail: TAIL.curl, head: "rotate(7deg)", ovals: "", accent: "question" },
  refused: { eyeL: EYE.flatL, eyeR: EYE.flatR, mouth: MOUTH.flat, pawL: PAW.stopL, pawR: PAW.stopR, tail: TAIL.curl, head: "none", ovals: "" },
  failed: { eyeL: EYE.sadL, eyeR: EYE.sadR, mouth: MOUTH.frown, pawL: PAW.foldL, pawR: PAW.foldR, tail: TAIL.down, head: "none", ovals: "", accent: "tear" },
  greeting: { eyeL: EYE.arcL, eyeR: EYE.arcR, mouth: MOUTH.shut, pawL: PAW.foldL, pawR: PAW.upR, tail: TAIL.curl, head: "none", ovals: "R" },
  done: { eyeL: EYE.arcL, eyeR: EYE.arcR, mouth: MOUTH.big, pawL: PAW.upL, pawR: PAW.upR, tail: TAIL.curl, head: "none", ovals: "LR", accent: "heart" },
};

/// The idle gestures: rare, short, and only in the quiet states. A gesture
/// is a fidget when it is noticed, so the timer below is tuned by how little
/// it is seen, never by how alive it feels (spec, section 3).
export type Gesture = "blink" | "ear" | "stretch" | "scratch" | "yawn";
export const GESTURES: readonly Gesture[] = ["blink", "blink", "ear", "stretch", "scratch", "yawn"];
/// The states in which an idle gesture may play.
export const IDLE_STATES: readonly CompanionState[] = ["resting", "attentive", "listening"];
/// The gap between gestures, in milliseconds: twenty to forty seconds.
export const IDLE_GAP_MIN = 20_000;
export const IDLE_GAP_MAX = 40_000;
/// The talking mouth shapes, cycled while an answer streams.
export const TALK_SHAPES = [MOUTH.open, MOUTH.half, MOUTH.small, MOUTH.open, MOUTH.half];
/// The one looping state stops itself after this long (spec, section 6).
export const THINKING_LOOP_MS = 5_000;
