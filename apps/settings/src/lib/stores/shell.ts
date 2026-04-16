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

export interface ShellConfig {
  toast?: ToastSection;
}

export const shell: ConfigStore<ShellConfig> =
  createConfigStore<ShellConfig>("shell");
