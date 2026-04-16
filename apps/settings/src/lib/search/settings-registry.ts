/// Settings registry.
///
/// Every user-visible setting is catalogued here once with:
///   * Human-readable title + description
///   * Search keywords
///   * The panel / section it lives in
///   * An optional inline-action definition so Waypointer can modify
///     the setting directly without opening the Settings app
///
/// This file is the single source of truth for both the in-app search
/// and the exported `settings-index.json` that Waypointer reads.

import type { PanelId } from "$lib/stores/navigation";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type InlineActionType = "toggle" | "select" | "slider";

export interface SelectOption {
  value: string;
  label: string;
}

export interface InlineAction {
  type: InlineActionType;
  /// Config file basename (e.g. `"appearance"`) — resolved against
  /// `~/.config/lunaris/{file}.toml` at execution time.
  configFile: string;
  /// Dot-notation key within the TOML file.
  configKey: string;
  /// For select actions.
  options?: SelectOption[];
  /// For slider actions.
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
}

export interface SettingDefinition {
  id: string;
  title: string;
  description: string;
  keywords: string[];
  panel: PanelId;
  section: string;
  /// Anchor fragment used in deep links. The frontend scrolls to the
  /// DOM element with `id={anchor}` and briefly highlights it.
  anchor: string;
  inlineAction?: InlineAction;
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

export const SETTINGS_REGISTRY: SettingDefinition[] = [
  // ── Appearance: Theme ──────────────────────────────────────────────
  {
    id: "appearance.theme.mode",
    title: "Theme Mode",
    description: "Switch between light and dark theme",
    keywords: ["dark", "light", "theme", "mode", "color scheme", "night"],
    panel: "appearance",
    section: "Theme",
    anchor: "theme-mode",
    inlineAction: {
      type: "select",
      configFile: "appearance",
      configKey: "theme.mode",
      options: [
        { value: "light", label: "Light" },
        { value: "dark", label: "Dark" },
      ],
    },
  },
  {
    id: "appearance.accent",
    title: "Accent Color",
    description: "Primary accent colour used across the shell and apps",
    keywords: ["accent", "color", "colour", "tint", "primary", "indigo", "blue"],
    panel: "appearance",
    section: "Theme",
    anchor: "accent-color",
  },

  // ── Appearance: Window ─────────────────────────────────────────────
  {
    id: "appearance.window.corner_radius",
    title: "Corner Radius",
    description: "Roundness of window corners, buttons, and cards",
    keywords: ["corner", "radius", "rounded", "round", "square", "border-radius"],
    panel: "appearance",
    section: "Window",
    anchor: "corner-radius",
    inlineAction: {
      type: "slider",
      configFile: "appearance",
      configKey: "window.corner_radius",
      min: 0,
      max: 16,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "appearance.window.border_width",
    title: "Border Width",
    description: "Thickness of window borders",
    keywords: ["border", "width", "thickness", "outline"],
    panel: "appearance",
    section: "Window",
    anchor: "border-width",
    inlineAction: {
      type: "slider",
      configFile: "appearance",
      configKey: "window.border_width",
      min: 0,
      max: 4,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "appearance.window.gaps",
    title: "Window Gaps",
    description: "Space between tiled windows",
    keywords: ["gap", "gaps", "space", "spacing", "tiling", "padding"],
    panel: "appearance",
    section: "Window",
    anchor: "gaps",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "layout.inner_gap",
      min: 0,
      max: 24,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "appearance.window.smart_gaps",
    title: "Smart Gaps",
    description: "Hide gaps when only one window is visible",
    keywords: ["smart", "gaps", "single", "auto"],
    panel: "appearance",
    section: "Window",
    anchor: "smart-gaps",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "layout.smart_gaps",
    },
  },

  // ── Appearance: Window Borders ─────────────────────────────────────
  {
    id: "appearance.window.border.focused",
    title: "Focused Window Border",
    description: "Color of the active window border",
    keywords: ["focus", "focused", "border", "active", "highlight", "window"],
    panel: "appearance",
    section: "Window Borders",
    anchor: "border-focused",
  },
  {
    id: "appearance.window.border.unfocused",
    title: "Unfocused Window Border",
    description: "Color of inactive window borders",
    keywords: ["unfocused", "inactive", "border", "window"],
    panel: "appearance",
    section: "Window Borders",
    anchor: "border-unfocused",
  },

  // ── Appearance: Typography ─────────────────────────────────────────
  {
    id: "appearance.fonts.interface",
    title: "Interface Font",
    description: "Font used for labels, menus, and UI text",
    keywords: ["font", "interface", "sans", "text", "typeface", "inter"],
    panel: "appearance",
    section: "Typography",
    anchor: "font-interface",
  },
  {
    id: "appearance.fonts.monospace",
    title: "Monospace Font",
    description: "Font used for code and terminal text",
    keywords: ["font", "mono", "monospace", "code", "terminal", "jetbrains"],
    panel: "appearance",
    section: "Typography",
    anchor: "font-monospace",
  },
  {
    id: "appearance.fonts.size",
    title: "Font Size",
    description: "Base font size for the interface",
    keywords: ["font", "size", "text", "large", "small", "zoom"],
    panel: "appearance",
    section: "Typography",
    anchor: "font-size",
    inlineAction: {
      type: "slider",
      configFile: "appearance",
      configKey: "fonts.size",
      min: 12,
      max: 18,
      step: 1,
      unit: "px",
    },
  },

  // ── Notifications: DND ─────────────────────────────────────────────
  {
    id: "notifications.dnd.mode",
    title: "Do Not Disturb",
    description: "Control which notifications can break through",
    keywords: [
      "dnd",
      "disturb",
      "quiet",
      "silent",
      "focus",
      "mute",
      "priority",
      "alarms",
    ],
    panel: "notifications",
    section: "Do Not Disturb",
    anchor: "dnd-mode",
    inlineAction: {
      type: "select",
      configFile: "notifications",
      configKey: "dnd.mode",
      options: [
        { value: "off", label: "Off" },
        { value: "priority", label: "Priority Only" },
        { value: "alarms", label: "Alarms Only" },
        { value: "total", label: "Total Silence" },
        { value: "scheduled", label: "Scheduled" },
      ],
    },
  },
  {
    id: "notifications.dnd.suppress_fullscreen",
    title: "Suppress in Fullscreen",
    description: "Queue notifications while a fullscreen app is active",
    keywords: ["fullscreen", "suppress", "quiet", "game", "video"],
    panel: "notifications",
    section: "Do Not Disturb",
    anchor: "suppress-fullscreen",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "dnd.suppress_fullscreen",
    },
  },

  // ── Notifications: Timing ──────────────────────────────────────────
  {
    id: "notifications.general.toast_duration_normal",
    title: "Toast Duration (Normal)",
    description: "How long normal notifications stay visible",
    keywords: ["toast", "duration", "time", "timeout", "notification"],
    panel: "notifications",
    section: "Timing",
    anchor: "toast-duration-normal",
    inlineAction: {
      type: "slider",
      configFile: "notifications",
      configKey: "general.toast_duration_normal",
      min: 1000,
      max: 15000,
      step: 500,
      unit: "ms",
    },
  },
  {
    id: "notifications.general.toast_duration_high",
    title: "Toast Duration (High Priority)",
    description: "How long high-priority notifications stay visible",
    keywords: ["toast", "duration", "high", "urgent", "important"],
    panel: "notifications",
    section: "Timing",
    anchor: "toast-duration-high",
  },
  {
    id: "notifications.general.max_visible_toasts",
    title: "Max Visible Toasts",
    description: "Maximum number of toasts shown at once",
    keywords: ["max", "visible", "toasts", "stack", "limit"],
    panel: "notifications",
    section: "Timing",
    anchor: "max-visible",
  },

  // ── Notifications: Toast Appearance ────────────────────────────────
  {
    id: "notifications.toast.position",
    title: "Toast Position",
    description: "Where notification toasts appear on screen",
    keywords: [
      "position",
      "toast",
      "location",
      "corner",
      "top",
      "bottom",
      "left",
      "right",
    ],
    panel: "notifications",
    section: "Toast Appearance",
    anchor: "toast-position",
    inlineAction: {
      type: "select",
      configFile: "shell",
      configKey: "toast.position",
      options: [
        { value: "top-right", label: "Top Right" },
        { value: "top-left", label: "Top Left" },
        { value: "top-center", label: "Top Center" },
        { value: "bottom-right", label: "Bottom Right" },
        { value: "bottom-left", label: "Bottom Left" },
      ],
    },
  },
  {
    id: "notifications.toast.animation",
    title: "Toast Animation",
    description: "Entry and exit animation for toasts",
    keywords: ["animation", "toast", "slide", "fade", "motion"],
    panel: "notifications",
    section: "Toast Appearance",
    anchor: "toast-animation",
    inlineAction: {
      type: "select",
      configFile: "shell",
      configKey: "toast.animation",
      options: [
        { value: "slide", label: "Slide" },
        { value: "fade", label: "Fade" },
        { value: "none", label: "None" },
      ],
    },
  },

  // ── Notifications: Grouping ────────────────────────────────────────
  {
    id: "notifications.grouping.by_app",
    title: "Group by App",
    description: "Group notifications by their source application",
    keywords: ["group", "app", "application", "bundle"],
    panel: "notifications",
    section: "Grouping",
    anchor: "group-by-app",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "grouping.by_app",
    },
  },
  {
    id: "notifications.grouping.stack_similar",
    title: "Stack Similar",
    description: "Merge near-duplicate notifications from the same app",
    keywords: ["stack", "similar", "merge", "deduplicate", "combine"],
    panel: "notifications",
    section: "Grouping",
    anchor: "stack-similar",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "grouping.stack_similar",
    },
  },

  // ── Notifications: History ─────────────────────────────────────────
  {
    id: "notifications.history.enabled",
    title: "Keep Notification History",
    description: "Store notifications in a history panel",
    keywords: ["history", "keep", "save", "log", "record", "notification"],
    panel: "notifications",
    section: "History",
    anchor: "history-enabled",
    inlineAction: {
      type: "toggle",
      configFile: "notifications",
      configKey: "history.enabled",
    },
  },
  {
    id: "notifications.history.max_age_days",
    title: "History Maximum Age",
    description: "Remove notifications older than this many days",
    keywords: ["history", "age", "days", "retention", "cleanup", "expire"],
    panel: "notifications",
    section: "History",
    anchor: "history-max-age",
  },
];
