/// Capability-gated host calls for Tier 2 iframe modules.
///
/// Each function wraps the postMessage protocol so module authors
/// never see raw envelopes. Calls return a Promise that rejects
/// with `HostError` on capability denial or unreachable host.
///
/// On the wire side every call goes shell → modulesd → policy check
/// → reply. Capability denial returns a typed `HostError` here so
/// modules can fall back gracefully.

import type {
  HostCall,
  HostReply,
  ModuleToHost,
  HostToModule,
  ErrorCode,
} from "./postmsg";

export class HostError extends Error {
  code: ErrorCode;
  constructor(code: ErrorCode, message: string) {
    super(message);
    this.code = code;
    this.name = "HostError";
  }
}

let nextRequestId = 1;
const pending = new Map<
  string,
  { resolve: (r: HostReply) => void; reject: (e: HostError) => void }
>();

/// Initialise the postMessage bridge. Must be called once at module
/// startup before any host call. Returns a promise that resolves on
/// the first `init` message from the shell.
export function bridgeInit(): Promise<{
  capabilities: import("./postmsg").Capabilities;
  theme: import("./postmsg").ThemeTokens;
}> {
  return new Promise((resolve) => {
    const handler = (ev: MessageEvent) => {
      const data = ev.data as HostToModule;
      if (data && data.type === "init") {
        window.removeEventListener("message", handler);
        resolve({ capabilities: data.capabilities, theme: data.theme });
      }
    };
    window.addEventListener("message", handler);
    sendUntyped({ type: "ready" });
  });
}

function sendUntyped(msg: ModuleToHost) {
  window.parent.postMessage(msg, "*");
}

function nextId(): string {
  return `req-${nextRequestId++}`;
}

function call(call: HostCall): Promise<HostReply> {
  return new Promise((resolve, reject) => {
    const requestId = nextId();
    pending.set(requestId, { resolve, reject });
    sendUntyped({ type: "host.call", requestId, call });
  });
}

// One global listener that demultiplexes host replies. Set up at
// module load; idempotent if `bridgeInit` is called multiple times.
window.addEventListener("message", (ev) => {
  const data = ev.data as HostToModule;
  if (!data || data.type !== "host.reply") return;
  const handler = pending.get(data.requestId);
  if (!handler) return;
  pending.delete(data.requestId);
  if (data.reply.type === "error") {
    handler.reject(new HostError(data.reply.code, data.reply.message));
  } else {
    handler.resolve(data.reply);
  }
});

export const graph = {
  /// Read Cypher query against the Knowledge Graph.
  /// Capability: `graph.read` allowlist on the queried namespace.
  async query(cypher: string): Promise<unknown[]> {
    const reply = await call({ type: "graph.query", cypher });
    if (reply.type !== "graph.result") {
      throw new HostError("internal", `unexpected reply: ${reply.type}`);
    }
    return JSON.parse(reply.rows);
  },

  /// Write Cypher.
  /// Capability: `graph.write` allowlist on the touched namespace.
  async write(cypher: string): Promise<unknown[]> {
    const reply = await call({ type: "graph.write", cypher });
    if (reply.type !== "graph.result") {
      throw new HostError("internal", `unexpected reply: ${reply.type}`);
    }
    return JSON.parse(reply.rows);
  },
};

export const network = {
  /// HTTP GET. URL host must be in the manifest's `network.allow`.
  async fetch(
    url: string,
    headers: Array<[string, string]> = [],
  ): Promise<{ status: number; body: Uint8Array }> {
    const reply = await call({ type: "network.fetch", url, headers });
    if (reply.type !== "network.body") {
      throw new HostError("internal", `unexpected reply: ${reply.type}`);
    }
    const binStr = atob(reply.bodyB64);
    const bytes = new Uint8Array(binStr.length);
    for (let i = 0; i < binStr.length; i++) bytes[i] = binStr.charCodeAt(i);
    return { status: reply.status, body: bytes };
  },
};

export const events = {
  /// Emit an event to the system Event Bus.
  /// Capability: `event_bus.publish` prefix allowlist.
  async emit(eventType: string, payload: Uint8Array): Promise<void> {
    const payloadB64 = btoa(String.fromCharCode(...payload));
    const reply = await call({ type: "events.emit", eventType, payloadB64 });
    if (reply.type !== "acked") {
      throw new HostError("internal", `unexpected reply: ${reply.type}`);
    }
  },
};
