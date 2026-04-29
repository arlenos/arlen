/// Workspaces & Tiling settings store.
///
/// Backed by `compositor.toml [workspaces]` + `[layout]`. Uses the
/// generic `createConfigStore<T>(file)` factory which writes through
/// the format-preserving `toml_writer` (Sprint A) so the user's
/// hand-edited comments stay intact.
///
/// Autotile is intentionally NOT exposed here. It's a runtime toggle
/// (compositor's QuickSettings / Waypointer flips it) and the
/// runtime-state-file `Option`-precedence (compositor #29 review
/// HIGH 2) makes a config-vs-state coordination tangle that we
/// don't need a Settings UI for. The toggle stays in QuickSettings.

import { createConfigStore, type ConfigStore } from "./config";

export type WorkspaceLayout = "Horizontal" | "Vertical";

export type WindowRuleAction = "float" | "tile";

/// Matcher half of a window rule. Mirrors the compositor's
/// `[[layout.window_rules]].match` table — see
/// `compositor/src/config/mod.rs::parse_layout_config`.
/// At least one field must be set or the rule won't match anything.
export interface WindowMatch {
  /// Regex matched against window app_id. Omit to match any.
  app_id?: string;
  /// Regex matched against window title. Omit to match any.
  title?: string;
  /// Match by window type (e.g. "dialog"). Omit to match any.
  window_type?: string;
}

export interface WindowRule {
  /// Match criteria — must be present, the compositor parser
  /// skips entries without it (Codex Sprint B review HIGH 2).
  match: WindowMatch;
  /// Force float or tile when the matcher hits.
  action: WindowRuleAction;
}

export interface LayoutConfig {
  inner_gap?: number;
  outer_gap?: number;
  smart_gaps?: boolean;
  tiled_headers?: boolean;
  window_rules?: WindowRule[];
}

export interface WorkspacesSection {
  workspace_layout?: WorkspaceLayout;
}

export interface CompositorConfig {
  workspaces?: WorkspacesSection;
  layout?: LayoutConfig;
  /// Other compositor.toml sections we don't render here are
  /// preserved as-is by toml_writer; the type declaration just
  /// avoids strict-mode complaints when reading them back.
  [key: string]: unknown;
}

export const compositor: ConfigStore<CompositorConfig> =
  createConfigStore<CompositorConfig>("compositor");

/// Defaults mirror compositor `LayoutConfig::default()` so the
/// reset-button behaviour matches the compositor source of truth.
export const LAYOUT_DEFAULTS = {
  inner_gap: 8,
  outer_gap: 8,
  smart_gaps: true,
  tiled_headers: false,
} as const;

export const WORKSPACE_LAYOUT_DEFAULT: WorkspaceLayout = "Horizontal";
