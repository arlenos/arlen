/// TypeScript types matching the Rust theme system structs.

/** Flat CSS variable set returned by `get_theme`. */
export interface CssVariables {
  variables: Record<string, string>;
  font_scale: number;
  variant: "dark" | "light";
}

/** Theme summary for the theme picker UI. */
export interface ThemeInfo {
  id: string;
  name: string;
  variant: "dark" | "light";
  is_builtin: boolean;
}

/** Full appearance config (mirrors `AppearanceConfig` in Rust). */
export interface AppearanceConfig {
  theme: { active: string };
  overrides: { accent?: string; font_scale?: number };
  accessibility: { reduce_motion: boolean };
}

/** CSS custom property names (without `--` prefix) for type-safe access. */
export const CSS_VARS = {
  // Background
  BG_SHELL: "color-bg-shell",
  BG_APP: "color-bg-app",
  BG_CARD: "color-bg-card",
  BG_OVERLAY: "color-bg-overlay",
  BG_INPUT: "color-bg-input",
  // Foreground
  FG_PRIMARY: "color-fg-primary",
  FG_SECONDARY: "color-fg-secondary",
  FG_DISABLED: "color-fg-disabled",
  FG_INVERSE: "color-fg-inverse",
  // Semantic
  ACCENT: "color-accent",
  ACCENT_HOVER: "color-accent-hover",
  ACCENT_PRESSED: "color-accent-pressed",
  SUCCESS: "color-success",
  WARNING: "color-warning",
  ERROR: "color-error",
  INFO: "color-info",
  // Border
  BORDER_DEFAULT: "color-border-default",
  BORDER_STRONG: "color-border-strong",
  // Radius
  RADIUS_SM: "radius-sm",
  RADIUS_MD: "radius-md",
  RADIUS_LG: "radius-lg",
  RADIUS_FULL: "radius-full",
  // Spacing
  SPACING_XS: "spacing-xs",
  SPACING_SM: "spacing-sm",
  SPACING_MD: "spacing-md",
  SPACING_LG: "spacing-lg",
  SPACING_XL: "spacing-xl",
  // Typography
  FONT_SANS: "font-sans",
  FONT_MONO: "font-mono",
  FONT_SIZE_BASE: "font-size-base",
  LINE_HEIGHT: "line-height",
  FONT_WEIGHT_NORMAL: "font-weight-normal",
  FONT_WEIGHT_MEDIUM: "font-weight-medium",
  FONT_WEIGHT_BOLD: "font-weight-bold",
  // Motion
  DURATION_FAST: "duration-fast",
  DURATION_NORMAL: "duration-normal",
  DURATION_SLOW: "duration-slow",
  EASING_DEFAULT: "easing-default",
  EASING_SPRING: "easing-spring",
  // Depth
  SHADOW_SM: "shadow-sm",
  SHADOW_MD: "shadow-md",
  SHADOW_LG: "shadow-lg",
} as const;
