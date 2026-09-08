/// Motion & Depth dimensions for that theme page: the transition durations +
/// easing, the reduce-motion switch, and the shadow elevation + blur. Same
/// override model as the other suite pages. Easing and shadows are chosen from
/// presets (the raw bezier / shadow strings are not hand-edited).
///
/// Mock-vs-live, corrected 9 September. **Reduce motion is live now**; the
/// durations, easing, shadows and blur are still local-only, so those sliders
/// change what the page draws and nothing else. The page says so on screen
/// rather than only here.
///
/// The old note said reduce motion was "a real command (`set_reduce_motion`)".
/// It is a real command IN THE SHELL, which Settings cannot call - a Tauri
/// command does not cross an app boundary - and the switch here called neither
/// it nor anything else. Settings' own path is the config key the shell reads,
/// `appearance.toml [accessibility] reduce_motion`, which is what it writes now.
/// Somebody who needs less animation flipped that switch and got none.
///
/// The rest wait on a decision rather than a wire: `theme_resolved_metrics`
/// reports every `motion.*` and `depth.*` key, but this page's `shadow` field is
/// ONE preset standing for four theme strings (`depth.shadow_{sm,md,lg,card}`),
/// and `easing` is a preset name where the theme carries a bezier. Whether the
/// page owns presets or the theme owns the strings is a design call, and wiring
/// the four that map cleanly while two stay inert would leave a page that works
/// until it does not.

import { writable, derived, get } from "svelte/store";

import { t } from "$lib/i18n/messages";
import { theme } from "$lib/stores/theme";

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
/// Derived, because these go straight into a generic segmented control as
/// `{value, label}` pairs: the control renders the label and has no business
/// knowing about the catalog, and a plain array would freeze at import.
export const easingPresets = derived(t, ($t) => [
  { value: "ease", label: $t("s.md.easing.ease") },
  { value: "linear", label: $t("s.md.easing.linear") },
  { value: "snappy", label: $t("s.md.easing.snappy") },
  { value: "spring", label: $t("s.md.easing.spring") },
]);
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
export const shadowPresets = derived(t, ($t) => [
  { value: "none", label: $t("s.md.shadow.none") },
  { value: "soft", label: $t("s.md.shadow.soft") },
  { value: "normal", label: $t("s.md.shadow.normal") },
  { value: "strong", label: $t("s.md.shadow.strong") },
]);
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
///
/// Reduce motion also goes to disk, because it is the one field here with a
/// reader. The store moves first so the switch does not lag, and a refused write
/// puts it back - a switch that stays where you left it while the file says
/// otherwise is the surface lying about what it did.
export async function setMd(key: string, value: string | number | boolean): Promise<void> {
  const before = get(overrides);
  overrides.update((o) => {
    const next = { ...o };
    if (value === MD_DEFAULTS[key]) delete next[key];
    else next[key] = value;
    return next;
  });
  if (key !== "reduceMotion") return;
  try {
    await theme.setValue("accessibility.reduce_motion", value);
  } catch (e) {
    overrides.set(before);
    throw e;
  }
}

/// Read what the config already holds, so the switch opens on the machine's
/// state rather than on the house default.
export async function loadMd(): Promise<void> {
  const held = get(theme).data?.accessibility?.reduce_motion;
  if (typeof held === "boolean" && held !== MD_DEFAULTS.reduceMotion) {
    overrides.update((o) => ({ ...o, reduceMotion: held }));
  }
}

/// Clear a field's override, back to the theme's value.
export function resetMd(key: string): void {
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
}
