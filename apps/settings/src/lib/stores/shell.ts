/// Shell config store.
///
/// Reads/writes `~/.config/lunaris/shell.toml`. The Settings app only
/// touches the `[toast]` section today — the desktop-shell has a live
/// watcher for this file (see `desktop-shell/src-tauri/src/shell_config.rs`)
/// which re-emits `lunaris://shell-config-changed` so toast appearance
/// changes propagate without a restart.

import { createConfigStore, type ConfigStore } from "./config";

export type ToastPosition =
  | "top-right"
  | "top-left"
  | "top-center"
  | "bottom-right"
  | "bottom-left";

export type ToastAnimation = "slide" | "fade" | "none";

export interface ToastSection {
  position?: ToastPosition;
  width?: number;
  animation?: ToastAnimation;
}

/// User-default settings for Focus Mode. Distinct from the runtime
/// `[focus]` section that desktop-shell uses for the active project
/// state — Sprint C added `[focus_settings]` so editing defaults
/// here doesn't trample the live state.
export interface FocusSettingsSection {
  /// Apps whose notifications get suppressed by default whenever
  /// any project's Focus Mode is active. Per-project `.project`
  /// `suppress_notifications_from` lists override this.
  default_suppressed_apps?: string[];
  /// Pin the active project name to the top bar.
  show_project_name?: boolean;
}

export const FOCUS_SETTINGS_DEFAULTS = {
  show_project_name: true,
} as const;

export interface ShellConfig {
  toast?: ToastSection;
  focus_settings?: FocusSettingsSection;
}

export const shell: ConfigStore<ShellConfig> =
  createConfigStore<ShellConfig>("shell");
