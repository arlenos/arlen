/// The theme colour roles for the Colors editor page (appearance-surface.md, the
/// granular theming suite). Each role shows the active theme's resolved value; a
/// per-field override layers on top (sparse: only overridden roles are written to
/// `theme.toml`). The store tracks resolved-vs-override so the page can render the
/// override mark + reset, and expose the effective colour for the live preview.
///
/// Mock-vs-live: reads a fixture (the dark house palette) until the coder exposes
/// the resolved per-role palette + the per-field override writes. The intended
/// bridge is flagged for the coder; switching/editing is affordance-only here.

import { writable, derived, get } from "svelte/store";

/// One semantic colour role.
export interface ColorRole {
  key: string;
  label: string;
  /// The one-line meaning, shown as the field's description.
  hint: string;
  /// `common` shows by default; `full` sits behind the "All roles" expander.
  tier: "common" | "full";
}

/// The 18 semantic colour roles (mirrors sdk/theme `ColorTokens`). The ~8 common
/// ones carry the everyday look; the rest are the full set behind an expander.
/// `label` and `hint` hold message KEYS, resolved with `$t` at the row: this is a
/// module-level table, so holding the text would pin it to the import-time locale.
export const COLOR_ROLES: ColorRole[] = [
  { key: "bg_app", label: "s.color.bg_app.label", hint: "s.color.bg_app.hint", tier: "common" },
  { key: "bg_card", label: "s.color.bg_card.label", hint: "s.color.bg_card.hint", tier: "common" },
  { key: "accent", label: "s.color.accent.label", hint: "s.color.accent.hint", tier: "common" },
  { key: "fg_primary", label: "s.color.fg_primary.label", hint: "s.color.fg_primary.hint", tier: "common" },
  { key: "border_default", label: "s.color.border_default.label", hint: "s.color.border_default.hint", tier: "common" },
  { key: "success", label: "s.color.success.label", hint: "s.color.success.hint", tier: "common" },
  { key: "warning", label: "s.color.warning.label", hint: "s.color.warning.hint", tier: "common" },
  { key: "error", label: "s.color.error.label", hint: "s.color.error.hint", tier: "common" },
  { key: "bg_shell", label: "s.color.bg_shell.label", hint: "s.color.bg_shell.hint", tier: "full" },
  { key: "bg_overlay", label: "s.color.bg_overlay.label", hint: "s.color.bg_overlay.hint", tier: "full" },
  { key: "bg_input", label: "s.color.bg_input.label", hint: "s.color.bg_input.hint", tier: "full" },
  { key: "fg_secondary", label: "s.color.fg_secondary.label", hint: "s.color.fg_secondary.hint", tier: "full" },
  { key: "fg_disabled", label: "s.color.fg_disabled.label", hint: "s.color.fg_disabled.hint", tier: "full" },
  { key: "fg_inverse", label: "s.color.fg_inverse.label", hint: "s.color.fg_inverse.hint", tier: "full" },
  { key: "accent_hover", label: "s.color.accent_hover.label", hint: "s.color.accent_hover.hint", tier: "full" },
  { key: "accent_pressed", label: "s.color.accent_pressed.label", hint: "s.color.accent_pressed.hint", tier: "full" },
  { key: "info", label: "s.color.info.label", hint: "s.color.info.hint", tier: "full" },
  { key: "border_strong", label: "s.color.border_strong.label", hint: "s.color.border_strong.hint", tier: "full" },
];

// The active theme's resolved values (fixture: the dark house palette).
const RESOLVED_FIXTURE: Record<string, string> = {
  bg_app: "#0f1115",
  bg_card: "#1a1d24",
  // The house default accent is monochrome: it follows the foreground (near-white
  // on dark), never a shipped colour. A colour appears only when the user picks
  // one; then accent/hover/pressed take that hue.
  accent: "#fafafa",
  fg_primary: "#e6e8ee",
  border_default: "#2a2f3a",
  success: "#16a34a",
  warning: "#ca8a04",
  error: "#dc2626",
  bg_shell: "#0b0d10",
  bg_overlay: "#21252e",
  bg_input: "#21252e",
  fg_secondary: "#a2a8b4",
  fg_disabled: "#6b7280",
  fg_inverse: "#0a0a12",
  accent_hover: "#ededed",
  accent_pressed: "#d6d6d6",
  info: "#2563eb",
  border_strong: "#3a404d",
};

/// The active theme's resolved colours, and the user's per-field overrides
/// (sparse: only edited roles appear).
export const resolved = writable<Record<string, string>>({ ...RESOLVED_FIXTURE });
export const overrides = writable<Record<string, string>>({});

/// The effective palette: an override wins, else the theme's resolved value.
export const effective = derived([resolved, overrides], ([$r, $o]) => {
  const out: Record<string, string> = { ...$r };
  for (const k of Object.keys($o)) out[k] = $o[k];
  return out;
});

/// Whether a role is currently overridden.
export function isOverridden(o: Record<string, string>, key: string): boolean {
  return key in o;
}

/// Set a per-field colour override.
export function setColorOverride(key: string, hex: string): void {
  overrides.update((o) => ({ ...o, [key]: hex }));
}

/// Clear a role's override, falling back to the theme's resolved value.
export function resetColorOverride(key: string): void {
  overrides.update((o) => {
    const next = { ...o };
    delete next[key];
    return next;
  });
}

// ── WCAG contrast, for the live check ────────────────────────────────────

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex);
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

function luminance(hex: string): number {
  const rgb = parseHex(hex);
  if (!rgb) return 0.5;
  const [r, g, b] = rgb.map((c) => {
    const x = c / 255;
    return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/// The WCAG contrast ratio between two hex colours (1..21).
export function contrastRatio(a: string, b: string): number {
  const la = luminance(a);
  const lb = luminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

/// The key legibility pairs to check live (foreground on its ground). Each is a
/// pair of role keys; the page renders the ratio + a pass/fail against 4.5:1.
export const CONTRAST_PAIRS: { labelKey: string; fg: string; bg: string }[] = [
  { labelKey: "s.contrast.onBackground", fg: "fg_primary", bg: "bg_app" },
  { labelKey: "s.contrast.onSurface", fg: "fg_primary", bg: "bg_card" },
  { labelKey: "s.contrast.onAccent", fg: "fg_inverse", bg: "accent" },
];

/// A rough hex validity check for the input.
export function isHex(v: string): boolean {
  return /^#?[0-9a-f]{6}$/i.test(v.trim());
}

/// Normalise to `#rrggbb` lowercase.
export function normHex(v: string): string {
  const t = v.trim().replace(/^#?/, "#");
  return t.toLowerCase();
}

/// Snapshot of overridden roles (for a "my overrides" filter / count).
export function overriddenKeys(): string[] {
  return Object.keys(get(overrides));
}
