/// The geometry dimensions for the Geometry theme page: roundness (the intensity
/// multiplier + the per-radius bases), window corners, spacing (a density
/// multiplier + the per-step bases), the tiling gaps and border width. Same
/// override model as the Colours page: each field shows the theme's resolved
/// value; an override layers on top (sparse). A master knob up front, the
/// granular per-token overrides behind expanders.
///
/// Mock-vs-live: `radius_intensity` / `border_width` / the compositor gaps are
/// real config keys, but the appearance/compositor stores don't render without a
/// backend, so this reads a fixture. The per-radius / window-corner / spacing
/// overrides need the theme.toml override backend (flagged for the coder).

import { writable, derived } from "svelte/store";

/// One editable geometry field.
export interface GeomField {
  key: string;
  label: string;
  hint: string;
  group: "roundness" | "window" | "spacing" | "gaps";
  /// `common` shows by default; `full` sits behind the group's expander.
  tier: "common" | "full";
  min: number;
  max: number;
  step: number;
  unit: string;
  /// Stored-to-displayed factor (intensity/density store a 1.0 multiplier, shown
  /// as a percentage). Default 1.
  scale?: number;
}

/// Every geometry field, grouped. Radii `full` is categorical and not editable.
export const GEOM_FIELDS: GeomField[] = [
  { key: "intensity", label: "Roundness", hint: "Rounds every corner at once", group: "roundness", tier: "common", min: 0, max: 200, step: 5, unit: "%", scale: 100 },
  { key: "r_chip", label: "Chip radius", hint: "Tags, badges, dots", group: "roundness", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "r_button", label: "Button radius", hint: "Buttons", group: "roundness", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "r_input", label: "Input radius", hint: "Text fields and selects", group: "roundness", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "r_card", label: "Card radius", hint: "Cards, popovers, panels", group: "roundness", tier: "full", min: 0, max: 32, step: 1, unit: "px" },
  { key: "r_modal", label: "Modal radius", hint: "Dialogs and sheets", group: "roundness", tier: "full", min: 0, max: 32, step: 1, unit: "px" },
  { key: "window_corner", label: "Window corners", hint: "The rounding of window corners", group: "window", tier: "common", min: 0, max: 24, step: 1, unit: "px" },
  { key: "border_width", label: "Border width", hint: "The window outline thickness", group: "window", tier: "common", min: 0, max: 4, step: 1, unit: "px" },
  { key: "density", label: "Density", hint: "Scales the space between things", group: "spacing", tier: "common", min: 50, max: 150, step: 5, unit: "%", scale: 100 },
  { key: "s_xs", label: "Space, tight", hint: "The tightest step", group: "spacing", tier: "full", min: 0, max: 16, step: 1, unit: "px" },
  { key: "s_sm", label: "Space, small", hint: "", group: "spacing", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "s_md", label: "Space, medium", hint: "", group: "spacing", tier: "full", min: 0, max: 40, step: 1, unit: "px" },
  { key: "s_lg", label: "Space, large", hint: "", group: "spacing", tier: "full", min: 0, max: 56, step: 1, unit: "px" },
  { key: "s_xl", label: "Space, widest", hint: "The widest step", group: "spacing", tier: "full", min: 0, max: 72, step: 1, unit: "px" },
  { key: "gap", label: "Gaps", hint: "Space between tiled windows", group: "gaps", tier: "common", min: 0, max: 24, step: 1, unit: "px" },
];

/// The active theme's resolved geometry values (fixture: the house defaults).
export const GEOM_DEFAULTS: Record<string, number> = {
  intensity: 1,
  r_chip: 4,
  r_button: 6,
  r_input: 8,
  r_card: 12,
  r_modal: 16,
  window_corner: 12,
  border_width: 2,
  density: 1,
  s_xs: 4,
  s_sm: 8,
  s_md: 16,
  s_lg: 24,
  s_xl: 32,
  gap: 8,
};

/// The user's per-field overrides (sparse: only edited fields), plus the smart
/// gaps toggle (a boolean, tracked separately).
export const overrides = writable<Record<string, number>>({});
export const smartGaps = writable(true);
export const smartGapsOverridden = writable(false);

/// The effective values: an override wins, else the resolved default.
export const effective = derived(overrides, ($o) => {
  const out: Record<string, number> = { ...GEOM_DEFAULTS };
  for (const k of Object.keys($o)) out[k] = $o[k];
  return out;
});

/// Whether a field is overridden.
export function isOverridden(o: Record<string, number>, key: string): boolean {
  return key in o;
}

/// Set a field; setting it back to the theme's value clears the override.
export function setGeom(key: string, value: number): void {
  overrides.update((o) => {
    const next = { ...o };
    if (value === GEOM_DEFAULTS[key]) delete next[key];
    else next[key] = value;
    return next;
  });
}

/// Clear a field's override, back to the theme's value.
export function resetGeom(key: string): void {
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
}

/// The effective radius CSS variables for the preview: each per-radius base
/// scaled by the intensity multiplier (the same `round(base × intensity)` the
/// runtime uses). Setting these on the preview container rounds it live.
export function previewRadiusVars(eff: Record<string, number>): string {
  const i = eff.intensity;
  const r = (base: number) => Math.round(base * i);
  return [
    `--radius-chip:${r(eff.r_chip)}px`,
    `--radius-button:${r(eff.r_button)}px`,
    `--radius-input:${r(eff.r_input)}px`,
    `--radius-card:${r(eff.r_card)}px`,
    `--radius-modal:${r(eff.r_modal)}px`,
  ].join(";");
}
