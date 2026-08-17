/// The typography dimensions for the Typography theme page: the interface + mono
/// font families, the base size, the line height, and the three weights. Same
/// override model as the other suite pages: each field shows the theme's resolved
/// value; an override layers on top (sparse).
///
/// Backed by the theme metric commands, not a fixture. `theme_resolved_metrics`
/// reports every dimension the resolved theme carries and `theme_set_metric`
/// writes `overrides.<key>` into the appearance config; the write command accepts
/// exactly what the read command reports, so the two cannot drift.
///
/// **Why the theme token and not `fonts.size`.** Two paths could carry a base
/// size: this one, which `sdk/theme` emits as `--font-size-base` and 108 CSS
/// declarations across the apps build their type scale from, and the appearance
/// config's `fonts.size`, which the Settings app applies to its OWN root element.
/// Only the first is live cross-app, so it is the canonical field; a slider
/// writing the other changes the size of Settings and nothing else.
///
/// This file was a fixture until 17 August, and said so - "line height + the
/// weights need the theme.toml override backend". That backend landed since, for
/// all seven fields, so the note outlived the gap it described.

import { invoke } from "@tauri-apps/api/core";
import { writable, derived, get } from "svelte/store";

/// The store's field names against the metric keys the backend reports.
const METRIC_KEY: Record<string, string> = {
  fontSans: "typography.font_sans",
  fontMono: "typography.font_mono",
  sizeBase: "typography.size_base",
  lineHeight: "typography.line_height",
  weightNormal: "typography.weight_normal",
  weightMedium: "typography.weight_medium",
  weightBold: "typography.weight_bold",
};

/// What the fields fall back to before the resolved theme has been read, and if
/// it cannot be read at all. The page renders the same either way; these are the
/// house defaults rather than an invented set.
export const TYPO_DEFAULTS: Record<string, string | number> = {
  fontSans: "Inter Variable",
  fontMono: "JetBrains Mono",
  sizeBase: 15,
  lineHeight: 1.5,
  weightNormal: 400,
  weightMedium: 500,
  weightBold: 700,
};

/// The active theme's resolved typography, once `load()` has read it.
export const resolved = writable<Record<string, string | number>>({ ...TYPO_DEFAULTS });

/// The user's per-field overrides (sparse: only edited fields).
export const overrides = writable<Record<string, string | number>>({});

/// The effective typography: an override wins, else the resolved value.
export const effective = derived([resolved, overrides], ([$r, $o]) => {
  const out: Record<string, string | number> = { ...$r };
  for (const k of Object.keys($o)) out[k] = $o[k];
  return out;
});

/// A metric string as the slider wants it: `"15px"` reads back as `15`.
function toNumberIfSized(key: string, raw: string): string | number {
  if (typeof TYPO_DEFAULTS[key] !== "number") return raw;
  const n = Number.parseFloat(raw);
  return Number.isFinite(n) ? n : (TYPO_DEFAULTS[key] as number);
}

/// The value the backend stores: a px size goes back as `"15px"`, so the theme
/// carries a CSS length rather than a bare number nothing can use.
function toMetric(key: string, value: string | number): string {
  if (typeof value !== "number") return String(value);
  return key === "sizeBase" ? `${value}px` : String(value);
}

/// Read the resolved theme's typography. Failure leaves the defaults in place:
/// the page still renders, and the row shows a value rather than an empty field.
export async function load(): Promise<void> {
  try {
    const metrics = await invoke<Record<string, string>>("theme_resolved_metrics");
    const next: Record<string, string | number> = { ...TYPO_DEFAULTS };
    for (const [field, key] of Object.entries(METRIC_KEY)) {
      const raw = metrics[key];
      if (raw !== undefined) next[field] = toNumberIfSized(field, raw);
    }
    resolved.set(next);
  } catch {
    // Keep the defaults; a theme that cannot be read is not a reason to blank
    // the page, and the write path reports its own failures.
  }
  await loadOverrides();
}

/// Read back the overrides already in the config.
///
/// Without this the rows showed the THEME's value and the system rendered the
/// override: set the text size to 22px, reopen Settings, and the row says 14 -
/// measured on 17 August by rendering the page against two seeded configs. The
/// page that writes the field was the one place not reading it, so it reported a
/// size the machine was not using and the reset button had nothing to clear.
async function loadOverrides(): Promise<void> {
  try {
    const stored = await invoke<Record<string, string> | null>("config_get", {
      file: "appearance",
      key: "overrides.typography",
    });
    if (!stored || typeof stored !== "object") return;
    const next: Record<string, string | number> = {};
    for (const [field, key] of Object.entries(METRIC_KEY)) {
      // `METRIC_KEY` holds the full dotted path; this table is already scoped to
      // `typography`, so only the leaf names it.
      const raw = stored[key.replace("typography.", "")];
      if (typeof raw === "string") next[field] = toNumberIfSized(field, raw);
    }
    overrides.set(next);
  } catch {
    // No overrides table, or a config that cannot be read. The rows then show
    // the theme's values, which is what an unoverridden field looks like.
  }
}

/// Set a field; setting it back to the theme's value clears the override.
///
/// The store updates first so the control does not lag the pointer, then the
/// write goes to the theme config. A failed write reverts the optimistic change,
/// because a slider that stays where you put it while the file says otherwise is
/// the surface lying about what it did.
export async function setTypo(key: string, value: string | number): Promise<void> {
  const before = get(overrides);
  const themeValue = get(resolved)[key];
  overrides.update((o) => {
    const next = { ...o };
    if (value === themeValue) delete next[key];
    else next[key] = value;
    return next;
  });
  const metric = METRIC_KEY[key];
  if (!metric) return;
  try {
    await invoke("theme_set_metric", { key: metric, value: toMetric(key, value) });
  } catch (e) {
    overrides.set(before);
    throw e;
  }
}

/// Clear a field's override, back to the theme's value.
///
/// Deletes the key rather than writing the theme's current value into it.
/// Writing it back would look identical today and pin the field: the next theme
/// with a different size would move every other dimension and leave this one
/// where the reset put it, which is the opposite of what the button says.
export async function resetTypo(key: string): Promise<void> {
  const before = get(overrides);
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
  const metric = METRIC_KEY[key];
  if (!metric) return;
  try {
    await invoke("config_reset", { file: "appearance", key: `overrides.${metric}` });
  } catch (e) {
    overrides.set(before);
    throw e;
  }
}

/// Whether a field is overridden.
export function isOverridden(o: Record<string, string | number>, key: string): boolean {
  return key in o;
}
