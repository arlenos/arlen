/// AI layer config store.
///
/// Reads/writes `~/.config/lunaris/ai.toml`. The `lunaris-ai-daemon`
/// watches this file: toggling `[ai] enabled` switches the AI layer
/// on/off live. `[ai] provider` is read by the daemon at startup, so
/// the AI page surfaces a restart hint for provider changes, the
/// same convention `graph.toml` uses.

import { createConfigStore, type ConfigStore } from "./config";

export interface AiSection {
  /// Whether the AI layer accepts queries. Off by default; the AI
  /// layer is opt-in (Foundation §5.1-5.2).
  enabled?: boolean;
  /// Catalogued provider name the daemon dispatches through. Phase
  /// 9-α ships only the local Ollama provider.
  provider?: string;
  /// Knowledge-Graph read tier, 0..=4 (Minimal/Session/Project/Time/Full).
  /// Out-of-range fails closed to Minimal in the daemon.
  access_level?: number;
  /// Baseline action mode: "suggest" | "supervised". Never "autonomous"
  /// globally (per-app only, via autonomous_apps).
  action_mode?: string;
  /// App ids allowed to act autonomously (per-app autonomy only).
  autonomous_apps?: string[];
}

/// `[agent]` section: the autonomous-agent behaviour controls.
export interface AgentSection {
  /// Enabled behaviour names.
  enabled?: string[];
  /// Allow safe deterministic curation workflows to write the graph
  /// (silent curator). Default off; the write still passes the full gate.
  executor_live?: boolean;
}

/// Optional `[provider]` section: model/window overrides for the daemon.
export interface ProviderSection {
  model?: string;
  context_window?: number;
}

export interface AiConfig {
  ai?: AiSection;
  agent?: AgentSection;
  provider?: ProviderSection;
}

export const ai: ConfigStore<AiConfig> = createConfigStore<AiConfig>("ai");
