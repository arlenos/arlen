/// Barrel export for the Lunaris theme system.

export type { CssVariables, ThemeInfo, AppearanceConfig } from "./types.js";
export { CSS_VARS } from "./types.js";
export { injectThemeVariables, getCssVar, setCssVar } from "./inject.js";
export {
  themeVars,
  availableThemes,
  activeThemeId,
  themeVariant,
  themeLoading,
  themeError,
  initTheme,
  setTheme,
  setAccentColor,
  setFontScale,
  setReduceMotion,
  resetTheme,
  getAppearanceConfig,
} from "./store.js";
