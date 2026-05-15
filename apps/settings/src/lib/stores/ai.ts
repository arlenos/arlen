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
}

export interface AiConfig {
  ai?: AiSection;
}

export const ai: ConfigStore<AiConfig> = createConfigStore<AiConfig>("ai");
