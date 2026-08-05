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
///
/// `label` and `hint` hold message KEYS, resolved with `$t` where the row renders:
/// a module-level table would capture the locale at import. An empty `hint` stays
/// empty - the row simply has no second line.
export const GEOM_FIELDS: GeomField[] = [
  { key: "intensity", label: "s.geom.intensity.label", hint: "s.geom.intensity.hint", group: "roundness", tier: "common", min: 0, max: 200, step: 5, unit: "%", scale: 100 },
  { key: "r_chip", label: "s.geom.r_chip.label", hint: "s.geom.r_chip.hint", group: "roundness", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "r_button", label: "s.geom.r_button.label", hint: "s.geom.r_button.hint", group: "roundness", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "r_input", label: "s.geom.r_input.label", hint: "s.geom.r_input.hint", group: "roundness", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "r_card", label: "s.geom.r_card.label", hint: "s.geom.r_card.hint", group: "roundness", tier: "full", min: 0, max: 32, step: 1, unit: "px" },
  { key: "r_modal", label: "s.geom.r_modal.label", hint: "s.geom.r_modal.hint", group: "roundness", tier: "full", min: 0, max: 32, step: 1, unit: "px" },
  { key: "window_corner", label: "s.geom.window_corner.label", hint: "s.geom.window_corner.hint", group: "window", tier: "common", min: 0, max: 24, step: 1, unit: "px" },
  { key: "border_width", label: "s.geom.border_width.label", hint: "s.geom.border_width.hint", group: "window", tier: "common", min: 0, max: 4, step: 1, unit: "px" },
  { key: "density", label: "s.geom.density.label", hint: "s.geom.density.hint", group: "spacing", tier: "common", min: 50, max: 150, step: 5, unit: "%", scale: 100 },
  { key: "s_xs", label: "s.geom.s_xs.label", hint: "s.geom.s_xs.hint", group: "spacing", tier: "full", min: 0, max: 16, step: 1, unit: "px" },
  { key: "s_sm", label: "s.geom.s_sm.label", hint: "", group: "spacing", tier: "full", min: 0, max: 24, step: 1, unit: "px" },
  { key: "s_md", label: "s.geom.s_md.label", hint: "", group: "spacing", tier: "full", min: 0, max: 40, step: 1, unit: "px" },
  { key: "s_lg", label: "s.geom.s_lg.label", hint: "", group: "spacing", tier: "full", min: 0, max: 56, step: 1, unit: "px" },
  { key: "s_xl", label: "s.geom.s_xl.label", hint: "s.geom.s_xl.hint", group: "spacing", tier: "full", min: 0, max: 72, step: 1, unit: "px" },
  { key: "gap", label: "s.geom.gap.label", hint: "s.geom.gap.hint", group: "gaps", tier: "common", min: 0, max: 24, step: 1, unit: "px" },
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
