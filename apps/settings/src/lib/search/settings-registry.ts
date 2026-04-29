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

  // ── Keyboard: layout ───────────────────────────────────────────────
  {
    id: "keyboard.layout",
    title: "Keyboard Layout",
    description: "Primary XKB layout (US, German, French, etc.)",
    keywords: ["layout", "xkb", "keymap", "language", "qwerty", "qwertz"],
    panel: "keyboard",
    section: "Layout",
    anchor: "search",
  },
  {
    id: "keyboard.repeat",
    title: "Key Repeat Rate",
    description: "How quickly a held key repeats",
    keywords: ["repeat", "rate", "delay", "autokey", "xkb"],
    panel: "keyboard",
    section: "Key Repeat",
    anchor: "search",
  },

  // ── Keyboard: shortcuts ────────────────────────────────────────────
  {
    id: "shortcuts.all",
    title: "Keyboard Shortcuts",
    description: "Rebind shortcuts for windows, workspaces, and apps",
    keywords: ["keybinding", "shortcut", "hotkey", "key", "bind", "keymap"],
    panel: "shortcuts",
    section: "Shortcuts",
    anchor: "search",
  },
  {
    id: "shortcuts.reset_all",
    title: "Reset Shortcuts to Defaults",
    description: "Restore every keybinding to its built-in default",
    keywords: ["reset", "default", "restore", "keybinding", "shortcut"],
    panel: "shortcuts",
    section: "Shortcuts",
    anchor: "search",
  },
  {
    id: "shortcuts.workspace",
    title: "Workspace Shortcuts",
    description: "Switch and move windows between workspaces",
    keywords: ["workspace", "desktop", "super", "switch", "move"],
    panel: "shortcuts",
    section: "Workspaces",
    anchor: "cat-workspace",
  },
  {
    id: "shortcuts.tiling",
    title: "Tiling Shortcuts",
    description: "Toggle tiling mode, monocle, scratchpad, and floating windows",
    keywords: ["tile", "tiling", "monocle", "float", "scratchpad"],
    panel: "shortcuts",
    section: "Tiling",
    anchor: "cat-tiling",
  },

  // ── Mouse ──────────────────────────────────────────────────────────
  {
    id: "mouse.acceleration",
    title: "Mouse Acceleration",
    description: "How much cursor speed scales with movement velocity",
    keywords: ["mouse", "pointer", "speed", "accel", "acceleration", "sensitivity"],
    panel: "mouse",
    section: "Behavior",
    anchor: "mouse-acceleration",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "mouse.acceleration",
      min: -1,
      max: 1,
      step: 0.1,
    },
  },
  {
    id: "mouse.natural_scroll",
    title: "Natural Scroll (Mouse)",
    description: "Scroll direction follows wheel movement",
    keywords: ["mouse", "scroll", "natural", "reverse", "direction"],
    panel: "mouse",
    section: "Behavior",
    anchor: "mouse-natural-scroll",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "mouse.natural_scroll",
    },
  },
  {
    id: "mouse.left_handed",
    title: "Left-Handed Mouse",
    description: "Swap left and right mouse buttons",
    keywords: ["mouse", "left", "handed", "button", "swap"],
    panel: "mouse",
    section: "Behavior",
    anchor: "mouse-left-handed",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "mouse.left_handed",
    },
  },

  // ── Touchpad ───────────────────────────────────────────────────────
  {
    id: "touchpad.tap_to_click",
    title: "Tap to Click",
    description: "Register taps on the touchpad as primary clicks",
    keywords: ["touchpad", "tap", "click", "trackpad"],
    panel: "touchpad",
    section: "Clicking",
    anchor: "touchpad-tap",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.tap_to_click",
    },
  },
  {
    id: "touchpad.natural_scroll",
    title: "Natural Scroll (Touchpad)",
    description: "Content follows finger direction",
    keywords: ["touchpad", "scroll", "natural", "trackpad"],
    panel: "touchpad",
    section: "Scrolling",
    anchor: "touchpad-natural-scroll",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.natural_scroll",
    },
  },
  {
    id: "touchpad.two_finger_scroll",
    title: "Two-Finger Scroll",
    description: "Scroll by dragging two fingers on the touchpad",
    keywords: ["touchpad", "two finger", "scroll", "trackpad"],
    panel: "touchpad",
    section: "Scrolling",
    anchor: "touchpad-two-finger",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.two_finger_scroll",
    },
  },
  {
    id: "touchpad.disable_while_typing",
    title: "Disable While Typing",
    description: "Ignore touchpad input briefly after each keystroke",
    keywords: ["touchpad", "typing", "disable", "palm", "rejection"],
    panel: "touchpad",
    section: "Clicking",
    anchor: "touchpad-dwt",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "touchpad.disable_while_typing",
    },
  },
  {
    id: "touchpad.acceleration",
    title: "Touchpad Acceleration",
    description: "How much cursor speed scales with finger velocity",
    keywords: ["touchpad", "accel", "acceleration", "speed", "trackpad"],
    panel: "touchpad",
    section: "Pointer",
    anchor: "touchpad-acceleration",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "touchpad.acceleration",
      min: -1,
      max: 1,
      step: 0.1,
    },
  },

  // ── Workspaces & Tiling (Sprint B) ─────────────────────────────────
  {
    id: "workspaces.layout",
    title: "Workspace Layout",
    description: "Horizontal or vertical workspace arrangement",
    keywords: [
      "workspace",
      "layout",
      "horizontal",
      "vertical",
      "direction",
      "arrange",
    ],
    panel: "workspaces",
    section: "Workspace Layout",
    anchor: "workspace-layout",
    inlineAction: {
      type: "select",
      configFile: "compositor",
      configKey: "workspaces.workspace_layout",
      options: [
        { value: "Horizontal", label: "Horizontal" },
        { value: "Vertical", label: "Vertical" },
      ],
    },
  },
  {
    id: "tiling.inner_gap",
    title: "Inner Gap",
    description: "Pixels between adjacent tiled windows",
    keywords: ["gap", "spacing", "tile", "tiling", "inner"],
    panel: "workspaces",
    section: "Tiling",
    anchor: "inner-gap",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "layout.inner_gap",
      min: 0,
      max: 32,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "tiling.outer_gap",
    title: "Outer Gap",
    description: "Pixels between tiled windows and the screen edge",
    keywords: ["gap", "spacing", "tile", "tiling", "outer", "margin"],
    panel: "workspaces",
    section: "Tiling",
    anchor: "outer-gap",
    inlineAction: {
      type: "slider",
      configFile: "compositor",
      configKey: "layout.outer_gap",
      min: 0,
      max: 32,
      step: 1,
      unit: "px",
    },
  },
  {
    id: "tiling.smart_gaps",
    title: "Smart Gaps",
    description: "Hide gaps when only one window is tiled",
    keywords: ["gap", "smart", "tile", "tiling", "single", "window"],
    panel: "workspaces",
    section: "Tiling",
    anchor: "smart-gaps",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "layout.smart_gaps",
    },
  },
  {
    id: "tiling.tiled_headers",
    title: "Tiled Window Headers",
    description: "Show window-control headers on single tiled windows",
    keywords: [
      "tiled",
      "headers",
      "title bar",
      "decoration",
      "i3",
      "sway",
      "hyprland",
    ],
    panel: "workspaces",
    section: "Tiling",
    anchor: "tiled-headers",
    inlineAction: {
      type: "toggle",
      configFile: "compositor",
      configKey: "layout.tiled_headers",
    },
  },
  {
    id: "tiling.window_rules",
    title: "Window Rules",
    description: "Force apps to float or tile based on app_id, title, or type",
    keywords: [
      "window",
      "rules",
      "float",
      "tile",
      "regex",
      "match",
      "exception",
      "app",
    ],
    panel: "workspaces",
    section: "Window Rules",
    anchor: "window-rules",
  },

  // ── System Actions (Sprint B) ──────────────────────────────────────
  // No inlineActions — system-action commands are free-form strings
  // that don't fit the toggle/select/slider model. Settings users
  // who want to change them open the panel via deepLink.
  {
    id: "system-actions.volume",
    title: "Volume Keys",
    description: "Volume up / down / mute / mute microphone commands",
    keywords: [
      "volume",
      "wpctl",
      "audio",
      "mute",
      "key",
      "fn",
      "hardware",
    ],
    panel: "system-actions",
    section: "Volume",
    anchor: "action-VolumeRaise",
  },
  {
    id: "system-actions.brightness",
    title: "Brightness Keys",
    description: "Brightness up / down commands for laptop Fn-row keys",
    keywords: [
      "brightness",
      "screen",
      "fn",
      "hardware",
      "key",
      "backlight",
      "laptop",
    ],
    panel: "system-actions",
    section: "Brightness",
    anchor: "action-BrightnessUp",
  },
  {
    id: "system-actions.media",
    title: "Media Keys",
    description: "Play / pause / next / previous track commands",
    keywords: [
      "media",
      "playerctl",
      "play",
      "pause",
      "next",
      "previous",
      "track",
      "music",
      "spotify",
    ],
    panel: "system-actions",
    section: "Media",
    anchor: "action-PlayPause",
  },
  {
    id: "system-actions.system",
    title: "System Keys",
    description:
      "Lock screen, suspend, power off, log out, launcher, screenshot",
    keywords: [
      "lock",
      "suspend",
      "power",
      "off",
      "logout",
      "launcher",
      "screenshot",
      "system",
    ],
    panel: "system-actions",
    section: "System",
    anchor: "action-LockScreen",
  },
];
