/// Knowledge-daemon `[projects]` config (`graph.toml`).
///
/// Used by the Focus Mode page (watch directories, recursion depth)
/// and the Knowledge Graph page (auto-promote threshold). The
/// daemon reads this file on startup only — Settings UI shows a
/// "restart Knowledge Daemon to apply" notice until live-reload
/// lands in a follow-up sprint.

import { createConfigStore, type ConfigStore } from "./config";

export interface ProjectsSection {
  watch_directories?: string[];
  max_depth?: number;
  auto_promote_threshold?: number;
}

export interface GraphConfig {
  projects?: ProjectsSection;
}

export const PROJECTS_DEFAULTS = {
  watch_directories: [
    "~/Projects",
    "~/Repositories",
    "~/Documents",
    "~/Developer",
    "~/Code",
  ],
  max_depth: 3,
  auto_promote_threshold: 3,
} as const;

export const graph: ConfigStore<GraphConfig> =
  createConfigStore<GraphConfig>("graph");
