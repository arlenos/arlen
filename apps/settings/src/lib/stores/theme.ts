/// Theme/Appearance store.
///
/// Writes the `~/.config/lunaris/appearance.toml` file the desktop-shell
/// theme watcher reads. The accent override lives in `[overrides].accent`
/// (this is what the shell reads -- `[colors].accent` is ignored by the
/// shell and therefore must NOT be used for the accent).
///
/// When no override is set, the accent defaults to the active theme's
/// built-in value from `desktop-shell/src-tauri/themes/{dark,light}.toml`:
///   - dark:  #6366f1
///   - light: #4f46e5

import { createConfigStore, type ConfigStore } from "./config";

export type ThemeMode = "light" | "dark";

export interface ThemeSection {
  /// Active theme id. Shell reads this for the built-in dark/light themes.
  active: string;
  /// High-level mode mirroring `active`.
  mode?: ThemeMode;
}

/// `[overrides]` section in the shell's appearance schema. This is where
/// the accent override belongs.
export interface OverridesSection {
  accent?: string;
  font_scale?: number;
}

export interface WindowBorderSection {
  /// Hex `#rrggbb[aa]` or the sentinel `"$accent"`.
  focused?: string;
  /// Hex `#rrggbb[aa]` or the sentinel `"$border"`.
  unfocused?: string;
}

export interface WindowSection {
  corner_radius?: number;
  border_width?: number;
  border?: WindowBorderSection;
}

/// Sentinel values understood by the compositor (and this app's UI).
export const BORDER_ACCENT_SENTINEL = "$accent";
export const BORDER_SUBTLE_SENTINEL = "$border";

export interface FontsSection {
  interface?: string;
  monospace?: string;
  size?: number;
}

export interface AccessibilitySection {
  reduce_motion?: boolean;
}

/// Full appearance.toml structure as seen by the Settings app.
///
/// `[window]` and `[fonts]` are settings-app specific sections -- the
/// shell does not read them yet. They are stored here so the Settings
/// app can maintain state for future shell support.
export interface AppearanceConfig {
  theme: ThemeSection;
  overrides?: OverridesSection;
  window?: WindowSection;
  fonts?: FontsSection;
  accessibility?: AccessibilitySection;
}

export const theme: ConfigStore<AppearanceConfig> =
  createConfigStore<AppearanceConfig>("appearance");

// ── Theme built-in accents ──────────────────────────────────────────────

/// Built-in accent for the Lunaris dark theme.
/// Matches `desktop-shell/src-tauri/themes/dark.toml`.
export const DARK_ACCENT = "#6366f1";

/// Built-in accent for the Lunaris light theme.
/// Matches `desktop-shell/src-tauri/themes/light.toml`.
export const LIGHT_ACCENT = "#4f46e5";

/// Monochrome foreground values per mode. When the user picks the
/// "Monochrome" swatch we store `MONO_SENTINEL` so the effective colour
/// follows the active theme mode instead of freezing a single hex.
export const MONO_DARK = "#fafafa";
export const MONO_LIGHT = "#171717";
export const MONO_SENTINEL = "$foreground";

/// Resolve the built-in default accent color for a theme mode.
/// This is what the shell renders when no override is set.
export function getThemeDefaultAccent(mode: string | undefined): string {
  return mode === "light" ? LIGHT_ACCENT : DARK_ACCENT;
}

export function getMonochromeAccent(mode: string | undefined): string {
  return mode === "light" ? MONO_LIGHT : MONO_DARK;
}

/// Resolve the current effective accent from a loaded config.
///   1. `overrides.accent` (user override, highest priority)
///      - `$foreground` sentinel -> mode-dependent monochrome
///   2. Theme default based on `theme.active`
export function resolveAccent(config: AppearanceConfig | null): string {
  if (!config) return DARK_ACCENT;
  const override = config.overrides?.accent;
  if (override === MONO_SENTINEL) return getMonochromeAccent(config.theme?.active);
  if (override) return override;
  return getThemeDefaultAccent(config.theme?.active);
}

/// Parse a `#rrggbb` string into its RGB components (0-255).
function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex);
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/// WCAG relative luminance in [0, 1].
function luminance(hex: string): number {
  const rgb = parseHex(hex);
  if (!rgb) return 0.5;
  const [r, g, b] = rgb.map((c) => {
    const x = c / 255;
    return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/// Return a contrasting foreground colour for the given accent hex.
/// Used for `--color-accent-foreground` so icons/text on the accent
/// surface stay legible regardless of which swatch the user picks.
export function accentForeground(hex: string): string {
  return luminance(hex) > 0.55 ? "#0a0a0a" : "#ffffff";
}

// ── CSS variable application ────────────────────────────────────────────

const DEFAULT_FONT_INTERFACE = "Inter Variable";
const DEFAULT_FONT_MONO = "JetBrains Mono";
const DEFAULT_FONT_SIZE = 14;
const DEFAULT_RADIUS = 8;

/// Apply a loaded AppearanceConfig to the document root as CSS variables.
///
/// Only sets the base design tokens (`--color-accent`, `--radius`,
/// `--font-sans`, ...). The shadcn names (`--primary`, `--accent`,
/// `--sidebar-primary`, ...) are derived from them in app.css via
/// `var()`, so a single assignment propagates everywhere.
export function applyAppearance(config: AppearanceConfig | null): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;

  // Apply theme mode first so the accent resolution that follows picks
  // up the right --color-fg-primary for the monochrome sentinel.
  const mode = config?.theme?.mode ?? config?.theme?.active ?? "dark";
  if (mode === "light") {
    root.dataset.theme = "light";
  } else {
    delete root.dataset.theme;
  }

  const accent = resolveAccent(config);
  root.style.setProperty("--color-accent", accent);
  root.style.setProperty("--color-accent-foreground", accentForeground(accent));

  const radius = config?.window?.corner_radius ?? DEFAULT_RADIUS;
  root.style.setProperty("--radius", `${radius / 16}rem`);

  const fontInterface =
    config?.fonts?.interface ?? DEFAULT_FONT_INTERFACE;
  const fontMono = config?.fonts?.monospace ?? DEFAULT_FONT_MONO;
  const fontSize = config?.fonts?.size ?? DEFAULT_FONT_SIZE;
  root.style.setProperty(
    "--font-sans",
    `"${fontInterface}", ui-sans-serif, system-ui, sans-serif`
  );
  root.style.setProperty(
    "--font-mono",
    `"${fontMono}", ui-monospace, monospace`
  );
  root.style.fontSize = `${fontSize}px`;

  console.log("[theme] applied:", {
    accent,
    radius,
    fontInterface,
    fontMono,
    fontSize,
    computed: getComputedStyle(root).getPropertyValue("--color-accent"),
  });
}

// Subscribe globally: any time the store data changes (load, set, reset,
// or config-watcher reload), re-apply the CSS variables.
theme.subscribe((state) => {
  if (state.data) {
    applyAppearance(state.data);
  }
});

/// Preset accent swatches shown in the UI. Value = hex, matches defaults
/// used by the desktop-shell built-in themes.
export const ACCENT_PRESETS: { value: string; name: string }[] = [
  { value: "#6366f1", name: "Indigo" },
  { value: "#3b82f6", name: "Blue" },
  { value: "#06b6d4", name: "Cyan" },
  { value: "#10b981", name: "Green" },
  { value: "#f59e0b", name: "Amber" },
  { value: "#ef4444", name: "Red" },
  { value: "#ec4899", name: "Pink" },
  { value: "#a855f7", name: "Purple" },
];

export const FONT_OPTIONS = [
  { value: "Inter Variable", label: "Inter" },
  { value: "system-ui", label: "System Default" },
  { value: "JetBrains Mono", label: "JetBrains Mono" },
  { value: "Fira Code", label: "Fira Code" },
];

export const MONO_FONT_OPTIONS = [
  { value: "JetBrains Mono", label: "JetBrains Mono" },
  { value: "Fira Code", label: "Fira Code" },
  { value: "ui-monospace", label: "System Mono" },
];
