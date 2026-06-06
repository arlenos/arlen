/// AI authorization prompt store.
///
/// The AI daemon asks the user to authorize an MCP action scope by
/// emitting a `ai://authorization-prompt` event (relayed from the
/// `org.lunaris.AI1` D-Bus signal by `ai_authz.rs`). This store
/// holds the active prompt and relays the user's decision back via
/// the `ai_respond_authorization` command.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface AuthorizationPrompt {
  /// Opaque identifier, echoed back with the decision.
  promptId: string;
  /// Scope label the daemon wants authorized.
  scope: string;
}

/// The prompt currently shown, or null when none is active.
export const current = writable<AuthorizationPrompt | null>(null);

let unlisten: UnlistenFn | null = null;

/// Start listening for authorization prompts. Mounted once globally.
export async function init(): Promise<void> {
  unlisten = await listen<AuthorizationPrompt>(
    "ai://authorization-prompt",
    (event) => current.set(event.payload),
  );
}

/// Stop listening.
export function dispose(): void {
  unlisten?.();
  unlisten = null;
}

/// Relay the user's decision to the AI daemon and clear the prompt.
export async function respond(
  promptId: string,
  granted: boolean,
): Promise<void> {
  current.set(null);
  try {
    await invoke("ai_respond_authorization", { promptId, granted });
  } catch (e) {
    console.error("[ai-authz] respond failed", e);
  }
}
