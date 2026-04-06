/// DOM injection of theme CSS variables.

import type { CssVariables } from "./types.js";

const STYLE_ID = "lunaris-theme-vars";

/** Inject (or update) a `<style>` element with all theme CSS variables. */
export function injectThemeVariables(cssVars: CssVariables): void {
  let style = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement("style");
    style.id = STYLE_ID;
    document.head.appendChild(style);
  }

  const lines: string[] = [":root {"];
  const keys = Object.keys(cssVars.variables).sort();
  for (const name of keys) {
    lines.push(`  --${name}: ${cssVars.variables[name]};`);
  }

  if (Math.abs(cssVars.font_scale - 1.0) > 0.001) {
    const px = Math.round(16 * cssVars.font_scale);
    lines.push(`  font-size: ${px}px;`);
  }

  lines.push("}");
  style.textContent = lines.join("\n");

  document.documentElement.style.colorScheme = cssVars.variant;
}

/** Read a computed CSS custom property value. */
export function getCssVar(name: string): string {
  return getComputedStyle(document.documentElement)
    .getPropertyValue(`--${name}`)
    .trim();
}

/** Set a single CSS custom property on `:root`. */
export function setCssVar(name: string, value: string): void {
  document.documentElement.style.setProperty(`--${name}`, value);
}
