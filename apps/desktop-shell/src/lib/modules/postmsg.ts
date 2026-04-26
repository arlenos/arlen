/// postMessage protocol used between the desktop-shell and a Tier 2
/// module iframe. Mirrors `modulesd-proto::HostCall` on the Rust side
/// and `@lunaris/module-sdk/postmsg.ts` on the module-author side.
///
/// The shell hosts the iframe, the daemon owns policy. Every host
/// call goes shell → modulesd over the Unix socket and the typed
/// reply comes back the same way.

export interface ThemeTokens {
  cssVars: Record<string, string>;
  variant: "dark" | "light";
}

export interface ProjectInfo {
  id: string;
  name: string;
  rootPath: string;
  tags: string[];
}

export interface Capabilities {
  network?: { allowedDomains: string[] };
  graph?: { read: string[]; write: string[] };
  eventBus?: { subscribe: string[]; publish: string[] };
  storage?: { quotaMb: number };
  notifications?: boolean;
  clipboard?: { read: boolean; write: boolean };
}

export type SearchAction =
  | { type: "copy"; text: string }
  | { type: "open_url"; url: string }
  | { type: "open_path"; path: string }
  | { type: "execute"; command: string }
  | { type: "custom"; handler: string; data: string };

export interface SearchResult {
  id: string;
  title: string;
  description?: string;
  icon?: string;
  relevance: number;
  action: SearchAction;
  pluginId?: string;
}

export type HostCall =
  | { type: "graph.query"; cypher: string }
  | { type: "graph.write"; cypher: string }
  | { type: "network.fetch"; url: string; headers: Array<[string, string]> }
  | { type: "events.emit"; eventType: string; payloadB64: string };

export type ErrorCode =
  | "not_found"
  | "permission_denied"
  | "module_failed"
  | "timeout"
  | "invalid_request"
  | "internal";

export type HostReply =
  | { type: "graph.result"; rows: string }
  | { type: "network.body"; status: number; bodyB64: string }
  | { type: "acked" }
  | { type: "error"; code: ErrorCode; message: string };

export type HostToModule =
  | { type: "init"; capabilities: Capabilities; theme: ThemeTokens }
  | { type: "theme.changed"; theme: ThemeTokens }
  | { type: "search"; requestId: string; query: string }
  | { type: "focus.activated"; project: ProjectInfo }
  | { type: "focus.deactivated" }
  | { type: "destroy" }
  | { type: "host.reply"; requestId: string; reply: HostReply };

export type ModuleToHost =
  | { type: "ready" }
  | { type: "search.results"; requestId: string; results: SearchResult[] }
  | { type: "indicator.update"; icon?: string; label?: string; badge?: string | number }
  | { type: "host.call"; requestId: string; call: HostCall }
  | { type: "module.error"; message: string };

/// Type guard: is this message from a Tier 2 module to the host?
export function isModuleToHost(data: unknown): data is ModuleToHost {
  return (
    typeof data === "object" &&
    data !== null &&
    "type" in data &&
    typeof (data as { type: unknown }).type === "string"
  );
}
