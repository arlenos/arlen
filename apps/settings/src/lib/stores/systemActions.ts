/// System Actions store + canonical defaults.
///
/// Mirrors `compositor::config::default_system_actions()` so the
/// reset-to-default behaviour is consistent. The list MUST stay in
/// sync with the compositor source — when a new System variant is
/// added there, mirror it here. Out-of-list actions still work in
/// compositor (compositor.toml [system_actions] passes any key
/// through, the dispatch ignores unknown system actions), but the
/// Settings UI won't show them.

import { compositor } from "./workspaces";
export { compositor };

/// Categories shown as section headers in the Settings page.
export type SystemActionCategory =
  | "Volume"
  | "Brightness"
  | "Media"
  | "System";

export interface SystemActionDef {
  /// `shortcuts::action::System` enum variant name as serialised in
  /// `compositor.toml [system_actions]`.
  key: string;
  /// User-facing label for the Settings row.
  label: string;
  /// Optional one-line description for the row.
  description?: string;
  category: SystemActionCategory;
  /// Built-in default command. Mirrors `default_system_actions()`
  /// in the compositor crate (compositor #29 / CC2).
  default: string;
}

export const SYSTEM_ACTIONS: SystemActionDef[] = [
  // ── Volume ────────────────────────────────────────────────────────
  {
    key: "VolumeRaise",
    label: "Volume up",
    category: "Volume",
    default: "spawn:wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+",
  },
  {
    key: "VolumeLower",
    label: "Volume down",
    category: "Volume",
    default: "spawn:wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-",
  },
  {
    key: "Mute",
    label: "Mute output",
    category: "Volume",
    default: "spawn:wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle",
  },
  {
    key: "MuteMic",
    label: "Mute microphone",
    category: "Volume",
    default: "spawn:wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle",
  },

  // ── Brightness ─────────────────────────────────────────────────────
  {
    key: "BrightnessUp",
    label: "Brightness up",
    description:
      "Routed through the shell so the gamma-corrected step worker handles it.",
    category: "Brightness",
    default: "shell:brightness_up",
  },
  {
    key: "BrightnessDown",
    label: "Brightness down",
    description:
      "Routed through the shell so the gamma-corrected step worker handles it.",
    category: "Brightness",
    default: "shell:brightness_down",
  },

  // ── Media ──────────────────────────────────────────────────────────
  {
    key: "PlayPause",
    label: "Play / pause",
    category: "Media",
    default: "spawn:playerctl play-pause",
  },
  {
    key: "PlayNext",
    label: "Next track",
    category: "Media",
    default: "spawn:playerctl next",
  },
  {
    key: "PlayPrev",
    label: "Previous track",
    category: "Media",
    default: "spawn:playerctl previous",
  },

  // ── System ─────────────────────────────────────────────────────────
  {
    key: "LockScreen",
    label: "Lock screen",
    category: "System",
    default: "spawn:loginctl lock-session",
  },
  {
    key: "Suspend",
    label: "Suspend",
    category: "System",
    default: "spawn:systemctl suspend",
  },
  {
    key: "PowerOff",
    label: "Power off",
    category: "System",
    default: "spawn:systemctl poweroff",
  },
  {
    key: "LogOut",
    label: "Log out",
    category: "System",
    default: "spawn:loginctl terminate-session $XDG_SESSION_ID",
  },
  {
    key: "HomeFolder",
    label: "Home folder",
    category: "System",
    default: "spawn:xdg-open ~",
  },
  {
    key: "WebBrowser",
    label: "Default browser",
    category: "System",
    default: "spawn:xdg-open https:",
  },
  {
    key: "Launcher",
    label: "Open launcher",
    category: "System",
    default: "shell:waypointer_open",
  },
  {
    key: "Screenshot",
    label: "Screenshot",
    category: "System",
    default: "spawn:grim",
  },
];

/// Display order — categories fixed, alphabetical within a category.
export const SYSTEM_ACTION_CATEGORIES: SystemActionCategory[] = [
  "Volume",
  "Brightness",
  "Media",
  "System",
];

export function actionsByCategory(
  category: SystemActionCategory,
): SystemActionDef[] {
  return SYSTEM_ACTIONS.filter((a) => a.category === category);
}
