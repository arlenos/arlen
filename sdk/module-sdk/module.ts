/// `createModule` entry point for Tier 2 iframe modules.
///
/// Module authors call this once at module top-level with their
/// extension-point handlers; the SDK wires up the postMessage
/// protocol, the host-call bridge, and the lifecycle hooks behind
/// the scenes.
///
/// Example
///
/// ```ts
/// import { createModule } from "@lunaris/module-sdk";
///
/// createModule({
///   id: "com.example.weather",
///   topbar: {
///     indicator: IndicatorComponent,
///     popover:   PopoverComponent,
///   },
/// });
/// ```

import type {
  Capabilities,
  HostToModule,
  SearchResult,
  ThemeTokens,
} from "./postmsg";
import { bridgeInit } from "./host";

export interface ModuleConfig {
  /// Reverse-domain module identifier; must match the manifest.
  id: string;

  /// Tier 1 stand-in: a search handler for `waypointer.search`.
  /// Tier 1 modules normally compile to WASM; this hook lets a
  /// Tier 2 (iframe) module also provide search results during dev,
  /// or for hybrid modules that ship both tiers.
  onSearch?(query: string): Promise<SearchResult[]> | SearchResult[];

  /// Topbar slot. The component types are intentionally `unknown` at
  /// this level because the SDK is framework-agnostic; bindings for
  /// Svelte/React etc. layer on top.
  topbar?: {
    indicator?: unknown;
    popover?: unknown;
  };

  /// Settings panel slot.
  settings?: {
    panel?: unknown;
  };

  /// Called when the shell pushes the active project context.
  onFocusActivated?(project: import("./postmsg").ProjectInfo): void;
  onFocusDeactivated?(): void;
}

export interface ModuleHandle {
  capabilities: Capabilities;
  theme: ThemeTokens;
}

/// Initialise the module. Returns a `ModuleHandle` once the shell
/// has sent the `init` message. After this resolves, host calls are
/// usable.
export async function createModule(config: ModuleConfig): Promise<ModuleHandle> {
  const handle = await bridgeInit();
  wireHandlers(config);
  return handle;
}

function wireHandlers(config: ModuleConfig) {
  window.addEventListener("message", async (ev) => {
    const data = ev.data as HostToModule;
    if (!data || typeof data !== "object") return;

    if (data.type === "search" && config.onSearch) {
      const results = await config.onSearch(data.query);
      window.parent.postMessage(
        {
          type: "search.results",
          requestId: data.requestId,
          results,
        },
        "*",
      );
      return;
    }

    if (data.type === "focus.activated" && config.onFocusActivated) {
      config.onFocusActivated(data.project);
      return;
    }

    if (data.type === "focus.deactivated" && config.onFocusDeactivated) {
      config.onFocusDeactivated();
      return;
    }
  });
}
