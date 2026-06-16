/**
 * Arlen theme loader.
 *
 * Fetches surface tokens from the Tauri backend (which reads theme.toml)
 * and applies them as CSS custom properties on :root. Call loadTheme()
 * once at app startup in +layout.svelte.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Theme v2: shared live-theming for every Arlen app (GAP-20)
// ---------------------------------------------------------------------------

/**
 * The resolved theme as a flat CSS-variable set, the wire shape the shell
 * (the theme authority) resolves and every app's theme consumer emits
 * identically. Mirrors the Rust `arlen_theme::css::CssVariables`.
 */
export interface CssVariables {
  /** Variable name (without the `--` prefix) to value. */
  variables: Record<string, string>;
  /** Font scale multiplier (1.0 = default). */
  font_scale: number;
  /** `"dark"` or `"light"` (matches CSS `color-scheme`). */
  variant: string;
}

const THEME_STYLE_ID = "arlen-theme-vars";

/**
 * Inject (or update) a single `<style>` element holding all theme CSS
 * variables on `:root`, plus the document `color-scheme`. Idempotent: each
 * call replaces the previous block.
 */
export function injectThemeVariables(css: CssVariables): void {
  let style = document.getElementById(THEME_STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement("style");
    style.id = THEME_STYLE_ID;
    document.head.appendChild(style);
  }

  const lines: string[] = [":root {"];
  for (const name of Object.keys(css.variables).sort()) {
    lines.push(`  --${name}: ${css.variables[name]};`);
  }
  if (Math.abs(css.font_scale - 1.0) > 0.001) {
    lines.push(`  font-size: ${Math.round(16 * css.font_scale)}px;`);
  }
  lines.push("}");
  style.textContent = lines.join("\n");

  document.documentElement.style.colorScheme = css.variant;
}

/**
 * Initialise live theming for a non-shell Arlen app. Reads the current theme
 * from the shell's broadcast (via the `arlen-shell` plugin's `theme_get`
 * command), injects it, then listens for `arlen://theme-v2-changed` (emitted
 * by the plugin's broadcast watcher) and re-injects, so a desktop-wide theme
 * switch live-reskins this app without a restart (GAP-20).
 *
 * Call once at app startup (`onMount` in the root `+layout.svelte`). The
 * returned function unsubscribes the listener. Requires the app to embed
 * `tauri_plugin_arlen_shell::init()` and grant `arlen-shell:allow-theme-get`.
 */
export async function initArlenTheme(): Promise<() => void> {
  try {
    const css = await invoke<CssVariables>("plugin:arlen-shell|theme_get");
    injectThemeVariables(css);
  } catch (e) {
    // No broadcast yet / plugin missing: keep the static stylesheet defaults.
    console.warn("initArlenTheme: initial theme load failed:", e);
  }

  const unlisten: UnlistenFn = await listen<CssVariables>(
    "arlen://theme-v2-changed",
    ({ payload }) => injectThemeVariables(payload),
  );
  return unlisten;
}

export interface SurfaceTokens {
  bgShell: string;
  bgApp: string;
  bgCard: string;
  bgOverlay: string;
  bgInput: string;
  fgShell: string;
  fgApp: string;
  accent: string;
  border: string;
  radius: string;
}

/**
 * Load surface tokens from the backend and apply them as CSS custom
 * properties on the document root.
 *
 * Safe to call multiple times (e.g. when theme.toml changes). Each call
 * overwrites the previous values.
 */
export async function loadTheme(): Promise<SurfaceTokens> {
  const tokens = await invoke<SurfaceTokens>("get_surface_tokens");
  applyTokens(tokens);
  return tokens;
}

/**
 * Apply surface tokens as CSS custom properties.
 *
 * Exported separately so tests can call it directly without a Tauri backend.
 */
export function applyTokens(tokens: SurfaceTokens): void {
  const root = document.documentElement;
  root.style.setProperty("--color-bg-shell", tokens.bgShell);
  root.style.setProperty("--color-bg-app", tokens.bgApp);
  root.style.setProperty("--color-bg-card", tokens.bgCard);
  root.style.setProperty("--color-bg-overlay", tokens.bgOverlay);
  root.style.setProperty("--color-bg-input", tokens.bgInput);
  root.style.setProperty("--color-fg-shell", tokens.fgShell);
  root.style.setProperty("--color-fg-app", tokens.fgApp);
  root.style.setProperty("--color-accent", tokens.accent);
  root.style.setProperty("--color-border", tokens.border);
  root.style.setProperty("--radius", tokens.radius);
}

/**
 * Built-in Panda theme tokens used as fallback in non-Tauri contexts
 * (e.g. Storybook, tests).
 */
export const PANDA_TOKENS: SurfaceTokens = {
  bgShell:   "#1a1a2e",
  bgApp:     "#ffffff",
  bgCard:    "#f5f5f7",
  bgOverlay: "#00000080",
  bgInput:   "#f0f0f0",
  fgShell:   "#e8e8f0",
  fgApp:     "#1a1a2e",
  accent:    "#0f0f0f",
  border:    "#e2e2e8",
  radius:    "0.5rem",
};
