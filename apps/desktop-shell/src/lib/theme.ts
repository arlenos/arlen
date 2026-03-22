/**
 * Lunaris theme loader.
 *
 * Fetches surface tokens from the Tauri backend (which reads theme.toml)
 * and applies them as CSS custom properties on :root. Call loadTheme()
 * once at app startup in +layout.svelte.
 */

import { invoke } from "@tauri-apps/api/core";

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
  bgShell:   "#09090b",
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
