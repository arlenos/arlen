/// The typography dimensions for the Typography theme page: the interface + mono
/// font families, the base size, the line height, and the three weights. Same
/// override model as the other suite pages: each field shows the theme's resolved
/// value; an override layers on top (sparse).
///
/// Mock-vs-live: `fonts.interface` / `fonts.monospace` / `fonts.size` are real
/// appearance config keys; line height + the weights need the theme.toml override
/// backend, and the font list is fixed (system-font enumeration via `fc-list` is
/// a coder gap). Fixture-backed until those land.

import { writable, derived } from "svelte/store";

/// The active theme's resolved typography (fixture: the house defaults).
export const TYPO_DEFAULTS: Record<string, string | number> = {
  fontSans: "Inter Variable",
  fontMono: "JetBrains Mono",
  sizeBase: 14,
  lineHeight: 1.5,
  weightNormal: 400,
  weightMedium: 500,
  weightBold: 700,
};

/// The user's per-field overrides (sparse: only edited fields).
export const overrides = writable<Record<string, string | number>>({});

/// The effective typography: an override wins, else the resolved default.
export const effective = derived(overrides, ($o) => {
  const out: Record<string, string | number> = { ...TYPO_DEFAULTS };
  for (const k of Object.keys($o)) out[k] = $o[k];
  return out;
});

/// Whether a field is overridden.
export function isOverridden(o: Record<string, string | number>, key: string): boolean {
  return key in o;
}

/// Set a field; setting it back to the theme's value clears the override.
export function setTypo(key: string, value: string | number): void {
  overrides.update((o) => {
    const next = { ...o };
    if (value === TYPO_DEFAULTS[key]) delete next[key];
    else next[key] = value;
    return next;
  });
}

/// Clear a field's override, back to the theme's value.
export function resetTypo(key: string): void {
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
}
