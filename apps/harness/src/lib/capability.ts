/// The AI capability context (ai-app.md §2.1): what the daemon enforces
/// from ai.toml, read via the shared `ai_capability` command. Both surfaces
/// render it — the conversation's capability strip and the agent dashboard's
/// posture banner — so the type lives here once.
import { invoke } from "@tauri-apps/api/core";

export interface Capability {
  /// The `[ai] enabled` master switch.
  enabled: boolean;
  /// The read tier the daemon enforces (from `access_level`).
  tier: string;
  /// The action mode (suggest / supervised / …).
  actionMode: string;
  /// The configured provider, when set.
  provider?: string | null;
  /// The configured model, when set.
  model?: string | null;
  /// Whether the agent executor writes (acting) or only proposes
  /// (suggest-only); the `[agent] executor_live` flag.
  executorLive: boolean;
}

/// Read the capability context; `null` when the read fails (AI layer
/// unreachable or unconfigured), which callers render honestly.
export async function readCapability(): Promise<Capability | null> {
  try {
    return await invoke<Capability>("ai_capability");
  } catch {
    return null;
  }
}
