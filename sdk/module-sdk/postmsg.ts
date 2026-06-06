/// postMessage wire protocol between a Tier 2 iframe and the
/// desktop-shell. Mirrors `modulesd-proto::HostCall` /
/// `modulesd-proto::HostReply` plus a few iframe-only events.
///
/// The shell forwards every `ModuleToHost` message to
/// `lunaris-modulesd` for capability gating; replies come back as
/// `HostToModule` messages tagged with the original `requestId`.

export type HostToModule =
  | { type: "init"; capabilities: Capabilities; theme: ThemeTokens }
  | { type: "search"; requestId: string; query: string }
  | { type: "focus.activated"; project: ProjectInfo }
  | { type: "focus.deactivated" }
  | { type: "host.reply"; requestId: string; reply: HostReply };

export type ModuleToHost =
  | { type: "ready" }
  | { type: "search.results"; requestId: string; results: SearchResult[] }
  | { type: "host.call"; requestId: string; call: HostCall };

export type HostCall =
  | { type: "graph.query"; cypher: string }
  | { type: "graph.write"; cypher: string }
  | { type: "network.fetch"; url: string; headers: Array<[string, string]> }
  | { type: "events.emit"; eventType: string; payloadB64: string };

export type HostReply =
  | { type: "graph.result"; rows: string }
  | { type: "network.body"; status: number; bodyB64: string }
  | { type: "acked" }
  | { type: "error"; code: ErrorCode; message: string };

export type ErrorCode =
  | "not_found"
  | "permission_denied"
  | "module_failed"
  | "timeout"
  | "invalid_request"
  | "internal";

export interface SearchResult {
  id: string;
  title: string;
  description?: string;
  icon?: string;
  relevance: number;
  action: SearchAction;
  pluginId?: string;
}

export type SearchAction =
  | { type: "copy"; text: string }
  | { type: "open_url"; url: string }
  | { type: "open_path"; path: string }
  | { type: "execute"; command: string }
  | { type: "custom"; handler: string; data: string };

export interface Capabilities {
  network?: { allowedDomains: string[] };
  graph?: { read: string[]; write: string[] };
  eventBus?: { subscribe: string[]; publish: string[] };
  storage?: { quotaMb: number };
  notifications?: boolean;
  clipboard?: { read: boolean; write: boolean };
}

export interface ThemeTokens {
  // Subset surfaced to modules; full set lives in shell theme/.
  colors: {
    accent: string;
    background: string;
    foreground: string;
    muted: string;
    card: string;
    border: string;
  };
  spacing: { unit: number };
  radius: { sm: number; md: number; lg: number };
}

export interface ProjectInfo {
  id: string;
  name: string;
  rootPath: string;
  tags: string[];
}
