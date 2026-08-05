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

/// `label` holds a message KEY, resolved with `$t` at the row: this table is
/// module-level, so storing the text would pin it to the locale at import.
export const SYSTEM_ACTIONS: SystemActionDef[] = [
  // ── Volume ────────────────────────────────────────────────────────
  {
    key: "VolumeRaise",
    label: "s.sysact.VolumeRaise",
    category: "Volume",
    default: "spawn:wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+",
  },
  {
    key: "VolumeLower",
    label: "s.sysact.VolumeLower",
    category: "Volume",
    default: "spawn:wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-",
  },
  {
    key: "Mute",
    label: "s.sysact.Mute",
    category: "Volume",
    default: "spawn:wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle",
  },
  {
    key: "MuteMic",
    label: "s.sysact.MuteMic",
    category: "Volume",
    default: "spawn:wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle",
  },

  // ── Brightness ─────────────────────────────────────────────────────
  {
    key: "BrightnessUp",
    label: "s.sysact.BrightnessUp",
    description:
      "Routed through the shell so the gamma-corrected step worker handles it.",
    category: "Brightness",
    default: "shell:brightness_up",
  },
  {
    key: "BrightnessDown",
    label: "s.sysact.BrightnessDown",
    description:
      "Routed through the shell so the gamma-corrected step worker handles it.",
    category: "Brightness",
    default: "shell:brightness_down",
  },

  // ── Media ──────────────────────────────────────────────────────────
  {
    key: "PlayPause",
    label: "s.sysact.PlayPause",
    category: "Media",
    default: "spawn:playerctl play-pause",
  },
  {
    key: "PlayNext",
    label: "s.sysact.PlayNext",
    category: "Media",
    default: "spawn:playerctl next",
  },
  {
    key: "PlayPrev",
    label: "s.sysact.PlayPrev",
    category: "Media",
    default: "spawn:playerctl previous",
  },

  // ── System ─────────────────────────────────────────────────────────
  {
    key: "LockScreen",
    label: "s.sysact.LockScreen",
    category: "System",
    default: "spawn:loginctl lock-session",
  },
  {
    key: "Suspend",
    label: "s.sysact.Suspend",
    category: "System",
    default: "spawn:systemctl suspend",
  },
  {
    key: "PowerOff",
    label: "s.sysact.PowerOff",
    category: "System",
    default: "spawn:systemctl poweroff",
  },
  {
    key: "LogOut",
    label: "s.sysact.LogOut",
    category: "System",
    default: "spawn:loginctl terminate-session $XDG_SESSION_ID",
  },
  {
    key: "HomeFolder",
    label: "s.sysact.HomeFolder",
    category: "System",
    default: "spawn:xdg-open ~",
  },
  {
    key: "WebBrowser",
    label: "s.sysact.WebBrowser",
    category: "System",
    default: "spawn:xdg-open https:",
  },
  {
    key: "Launcher",
    label: "s.sysact.Launcher",
    category: "System",
    default: "shell:waypointer_open",
  },
  {
    key: "Screenshot",
    label: "s.sysact.Screenshot",
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
