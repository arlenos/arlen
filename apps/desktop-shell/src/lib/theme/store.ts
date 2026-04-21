/// Svelte stores and async actions for the theme system.

import { derived, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { injectThemeVariables } from "./inject.js";
import type { AppearanceConfig, CssVariables, ThemeInfo } from "./types.js";

// ---------------------------------------------------------------------------
// Stores
// ---------------------------------------------------------------------------

/** Current resolved CSS variables (null until first load). */
export const themeVars = writable<CssVariables | null>(null);

/** All available themes (built-in + user). */
export const availableThemes = writable<ThemeInfo[]>([]);

/** Currently active theme ID. */
export const activeThemeId = writable<string>("dark");

/** Current color scheme derived from themeVars. */
export const themeVariant = derived(themeVars, ($v) => $v?.variant ?? "dark");

/** Whether a theme operation is in progress. */
export const themeLoading = writable<boolean>(false);

/** Last error message (null = no error). */
export const themeError = writable<string | null>(null);

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/** Initialize the theme system: load current theme, inject CSS, start listener. */
export async function initTheme(): Promise<void> {
  themeLoading.set(true);
  themeError.set(null);

  try {
    // Three independent Tauri calls — run in parallel. Previously
    // sequential, which blocked first paint for ~300-500ms because the
    // CSS injection can't happen until `get_theme` resolves.
    const [css, themes, id] = await Promise.all([
      invoke<CssVariables>("get_theme"),
      invoke<ThemeInfo[]>("get_available_themes"),
      invoke<string>("get_active_theme_id"),
    ]);
    injectThemeVariables(css);
    themeVars.set(css);
    availableThemes.set(themes);
    activeThemeId.set(id);
  } catch (e) {
    themeError.set(e instanceof Error ? e.message : String(e));
  }

  themeLoading.set(false);

  // Live updates from Rust (theme switch, accent change, etc.).
  listen<CssVariables>("lunaris://theme-v2-changed", ({ payload }) => {
    injectThemeVariables(payload);
    themeVars.set(payload);
  });
}

/** Switch to a different theme by ID. */
export async function setTheme(id: string): Promise<void> {
  themeLoading.set(true);
  themeError.set(null);
  try {
    const css = await invoke<CssVariables>("set_theme", { id });
    injectThemeVariables(css);
    themeVars.set(css);
    activeThemeId.set(id);
  } catch (e) {
    themeError.set(e instanceof Error ? e.message : String(e));
  }
  themeLoading.set(false);
}

/** Set a custom accent color override (empty string to reset). */
export async function setAccentColor(color: string): Promise<void> {
  themeError.set(null);
  try {
    const css = await invoke<CssVariables>("set_accent_color", { color });
    injectThemeVariables(css);
    themeVars.set(css);
  } catch (e) {
    themeError.set(e instanceof Error ? e.message : String(e));
  }
}

/** Set the font scale multiplier (0.5 - 2.0, 1.0 = default). */
export async function setFontScale(scale: number): Promise<void> {
  themeError.set(null);
  try {
    const css = await invoke<CssVariables>("set_font_scale", { scale });
    injectThemeVariables(css);
    themeVars.set(css);
  } catch (e) {
    themeError.set(e instanceof Error ? e.message : String(e));
  }
}

/** Toggle the reduce-motion accessibility setting. */
export async function setReduceMotion(enabled: boolean): Promise<void> {
  themeError.set(null);
  try {
    const css = await invoke<CssVariables>("set_reduce_motion", { enabled });
    injectThemeVariables(css);
    themeVars.set(css);
  } catch (e) {
    themeError.set(e instanceof Error ? e.message : String(e));
  }
}

/** Reset all theme settings to defaults. */
export async function resetTheme(): Promise<void> {
  themeLoading.set(true);
  themeError.set(null);
  try {
    const css = await invoke<CssVariables>("reset_theme");
    injectThemeVariables(css);
    themeVars.set(css);
    activeThemeId.set("dark");
  } catch (e) {
    themeError.set(e instanceof Error ? e.message : String(e));
  }
  themeLoading.set(false);
}

/** Get the full appearance config from Rust. */
export async function getAppearanceConfig(): Promise<AppearanceConfig | null> {
  try {
    return await invoke<AppearanceConfig>("get_appearance_config");
  } catch {
    return null;
  }
}
