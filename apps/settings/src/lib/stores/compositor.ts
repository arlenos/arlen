/// Compositor config store.
///
/// Reads/writes `~/.config/lunaris/compositor.toml`. Today the Settings
/// app only uses the `[layout]` section (gaps + smart_gaps), because the
/// compositor already has a live file watcher for that file — writing
/// here is the same path the desktop-shell's LayoutPopover uses.
///
/// Other compositor TOML sections (keybindings, xkb, workspaces) are
/// left untouched by the Settings app for now.

import { createConfigStore, type ConfigStore } from "./config";

export interface LayoutSection {
  /// Inner gap between tiled windows (px).
  inner_gap?: number;
  /// Outer gap between tiled windows and the screen edge (px).
  outer_gap?: number;
  /// Collapse outer gaps when only one window is visible.
  smart_gaps?: boolean;
}

export interface CompositorConfig {
  layout?: LayoutSection;
}

export const compositor: ConfigStore<CompositorConfig> =
  createConfigStore<CompositorConfig>("compositor");
