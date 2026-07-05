/// Motion & Depth dimensions for that theme page: the transition durations +
/// easing, the reduce-motion switch, and the shadow elevation + blur. Same
/// override model as the other suite pages. Easing and shadows are chosen from
/// presets (the raw bezier / shadow strings are not hand-edited).
///
/// Mock-vs-live: `reduce_motion` is a real command (`set_reduce_motion`); the
/// durations / easing / shadows / blur need the theme.toml override backend.
/// Fixture-backed until those land.

import { writable, derived } from "svelte/store";

/// The active theme's resolved values (fixture: the house defaults).
export const MD_DEFAULTS: Record<string, string | number | boolean> = {
  durationFast: 100,
  durationNormal: 200,
  durationSlow: 400,
  easing: "ease",
  shadow: "normal",
  blurEnabled: false,
  reduceMotion: false,
};

/// Easing presets → the cubic-bezier the runtime uses.
export const EASING_PRESETS = [
  { value: "ease", label: "Ease" },
  { value: "linear", label: "Linear" },
  { value: "snappy", label: "Snappy" },
  { value: "spring", label: "Spring" },
];
const EASING_BEZIER: Record<string, string> = {
  ease: "cubic-bezier(0.4, 0, 0.2, 1)",
  linear: "linear",
  snappy: "cubic-bezier(0.2, 0, 0, 1)",
  spring: "cubic-bezier(0.34, 1.56, 0.64, 1)",
};
/// The CSS timing-function for an easing preset key.
export function easingBezier(key: string): string {
  return EASING_BEZIER[key] ?? EASING_BEZIER.ease;
}

/// Shadow elevation presets → a CSS box-shadow.
export const SHADOW_PRESETS = [
  { value: "none", label: "None" },
  { value: "soft", label: "Soft" },
  { value: "normal", label: "Normal" },
  { value: "strong", label: "Strong" },
];
const SHADOW_CSS: Record<string, string> = {
  none: "none",
  soft: "0 1px 3px rgba(0, 0, 0, 0.28)",
  normal: "0 6px 16px rgba(0, 0, 0, 0.38)",
  strong: "0 16px 40px rgba(0, 0, 0, 0.55)",
};
/// The CSS box-shadow for a shadow preset key.
export function shadowCss(key: string): string {
  return SHADOW_CSS[key] ?? SHADOW_CSS.normal;
}

/// The user's per-field overrides (sparse: only edited fields).
export const overrides = writable<Record<string, string | number | boolean>>({});

/// The effective values: an override wins, else the resolved default.
export const effective = derived(overrides, ($o) => {
  const out: Record<string, string | number | boolean> = { ...MD_DEFAULTS };
  for (const k of Object.keys($o)) out[k] = $o[k];
  return out;
});

/// Whether a field is overridden.
export function isOverridden(o: Record<string, string | number | boolean>, key: string): boolean {
  return key in o;
}

/// Set a field; setting it back to the theme's value clears the override.
export function setMd(key: string, value: string | number | boolean): void {
  overrides.update((o) => {
    const next = { ...o };
    if (value === MD_DEFAULTS[key]) delete next[key];
    else next[key] = value;
    return next;
  });
}

/// Clear a field's override, back to the theme's value.
export function resetMd(key: string): void {
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
}
