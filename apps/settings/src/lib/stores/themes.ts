/// The theme gallery (appearance-surface.md APP-R2): the installed themes, the
/// active one, and switching between them. A theme is one palette applied
/// everywhere (no light/dark mode; theme-system.md, Tim 30 June).
///
/// Mock-vs-live: `get_available_themes` / `set_theme` exist in the desktop-shell
/// but not yet in settings, and `ThemeInfo` carries no preview colours, so this
/// reads a fixture (with known palettes for the swatch strip) until the coder
/// bridges those. Switching the active theme persists for real through the
/// appearance config (`theme.active`); the live re-apply rides the `set_theme`
/// bridge when it lands.

import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";
import { theme } from "./theme";

/// One installed theme as the gallery shows it.
export interface ThemeInfo {
  id: string;
  name: string;
  /// A bundled theme (cannot be removed), vs a user-installed/imported one.
  isBuiltin: boolean;
  /// A few representative colours (bg, surface, accent, secondary, fg) for the
  /// preview strip. The backend `ThemeInfo` has none yet; the coder resolves the
  /// palette per theme later, the fixture carries known ones.
  swatch: string[];
}

/// The installed themes, the active id, and whether the load has run.
export const themes = writable<ThemeInfo[]>([]);
export const activeThemeId = writable<string>("arlen-dark");
export const themesLoaded = writable(false);

/// The active theme's full record (for the Appearance page's summary row).
export const activeTheme = derived([themes, activeThemeId], ([$themes, $id]) =>
  $themes.find((t) => t.id === $id) ?? $themes[0] ?? null,
);

// Fixture: the built-in Arlen Dark plus a few community schemes as imported
// examples, each with its signature palette for the preview.
const FIXTURE: ThemeInfo[] = [
  { id: "arlen-dark", name: "Arlen Dark", isBuiltin: true, swatch: ["#0f1115", "#1a1d24", "#7c93ff", "#2a2f3a", "#e6e8ee"] },
  { id: "nord", name: "Nord", isBuiltin: false, swatch: ["#2e3440", "#3b4252", "#88c0d0", "#434c5e", "#eceff4"] },
  { id: "gruvbox-dark", name: "Gruvbox Dark", isBuiltin: false, swatch: ["#282828", "#3c3836", "#fabd2f", "#504945", "#ebdbb2"] },
  { id: "catppuccin-mocha", name: "Catppuccin Mocha", isBuiltin: false, swatch: ["#1e1e2e", "#313244", "#cba6f7", "#45475a", "#cdd6f4"] },
  { id: "tokyo-night", name: "Tokyo Night", isBuiltin: false, swatch: ["#1a1b26", "#24283b", "#7aa2f7", "#414868", "#c0caf5"] },
];

/// Load the installed themes + the active id. Prefers the real bridge; falls
/// back to the fixture while the settings-side commands are unwired.
export async function loadThemes(): Promise<void> {
  try {
    const list = await invoke<{ id: string; name: string; is_builtin: boolean }[]>("get_available_themes");
    themes.set(list.map((t) => ({ id: t.id, name: t.name, isBuiltin: t.is_builtin, swatch: [] })));
    activeThemeId.set(await invoke<string>("get_active_theme_id"));
  } catch {
    themes.set(FIXTURE);
  } finally {
    themesLoaded.set(true);
  }
}

/// Make a theme the active one. Persists the id to the appearance config for
/// real; the live re-apply rides the shell's `set_theme` bridge when present.
export async function setActiveTheme(id: string): Promise<void> {
  activeThemeId.set(id);
  try {
    await theme.setValue("theme.active", id);
    await invoke("set_theme", { id });
  } catch {
    // The persist path already ran; live re-apply lands with the bridge.
  }
}

/// Install a theme from a file on disk (a validated copy into the themes dir).
/// Mock until the coder's file-picker + validate command lands.
export async function installThemeFile(): Promise<void> {
  try {
    await invoke("theme_install_file");
    await loadThemes();
  } catch {
    // No command yet; the affordance is present for when it lands.
  }
}

/// Import a community scheme (base16 / Catppuccin) via the inbound adapters.
/// Mock until the adapter bridge lands.
export async function importScheme(kind: "base16" | "catppuccin"): Promise<void> {
  try {
    await invoke("theme_import_scheme", { kind });
    await loadThemes();
  } catch {
    // No command yet.
  }
}
