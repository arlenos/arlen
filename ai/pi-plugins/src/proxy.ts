// The Arlen KG/OS proxy-tools plugin: registers privileged tools (graph.read,
// graph.write, ... OS mutations as they land) as pi custom tools whose execute()
// does NOT touch the KG/OS inside pi. Each forwards to the daemon's Execute verb,
// which runs the real action in trusted Rust, re-validates the args server-side,
// audits, registers compensation, and returns only the result. pi holds no KG
// socket and no OS right - it can only ask (pi-agent-adoption.md §A, the two-tool
// classes: structural no-ambient-authority + anti-Recall).
//
// Fail-closed: a daemon-unreachable, unexpected-reply, or error outcome surfaces
// as a tool error to the model, never a silent success. Today the daemon's Execute
// runners are fail-closed (DeniedRunner/DeniedWriter) pending the gated cutovers,
// so a proxy tool currently returns a PermissionDenied error - the MECHANISM is
// here and wire-proven; it gains real capability when the live runners land. The
// real-pi wiring (registerTool with a TypeBox parameter schema) is the separate
// extension-entry step that pairs with that cutover; this models the minimal
// registerTool surface so the package stays free of a pi-package dependency.
//
// As in the gate/audit plugins, the args carried to Execute are NOT trusted: the
// daemon re-validates every privileged argument server-side (footgun 1b).

import { calls, ContractClient, type Reply } from "./contract.js";

/** A content block in a tool result (only `text` is produced here). */
export interface ContentBlock {
  type: string;
  text?: string;
  [key: string]: unknown;
}

/** pi's tool-execute result shape (the subset the proxy produces). */
export interface ProxyToolResult {
  content: ContentBlock[];
  details?: unknown;
  isError?: boolean;
}

/** The subset of pi's tool definition the proxy registers. `parameters` is a
 *  JSON-schema object (a permissive default here; the daemon re-validates), kept
 *  as `unknown` so the package needs no TypeBox dependency. */
export interface ToolDefinition {
  name: string;
  label: string;
  description: string;
  parameters: unknown;
  execute(
    toolCallId: string,
    params: Record<string, unknown>,
    ...rest: unknown[]
  ): Promise<ProxyToolResult>;
}

/** The minimal pi `ExtensionAPI` surface the proxy registers on. */
export interface ProxyExtensionAPI {
  registerTool(def: ToolDefinition): void;
}

/** Just the `call` method, so a test can inject a mock client. */
export type CallClient = Pick<ContractClient, "call">;

/** One privileged proxy tool the model may call (its effect runs daemon-side). */
export interface ProxyToolSpec {
  name: string;
  label: string;
  description: string;
}

/** Proxy-tools configuration (injectable for tests). */
export interface ProxyOptions {
  /** How to obtain a contract client; defaults to connecting the real socket. */
  connect?: () => Promise<CallClient>;
  /** Which privileged tools to register; defaults to the KG read/write pair. */
  tools?: ProxyToolSpec[];
}

/** A permissive JSON-schema object for a proxy tool's parameters: the model may
 *  pass any object, since the daemon re-validates the privileged args itself. */
const PERMISSIVE_PARAMETERS = { type: "object", additionalProperties: true } as const;

/** The default privileged tools: the KG read/write proxies (OS mutation proxies
 *  land as their daemon-side executors do). */
export const DEFAULT_PROXY_TOOLS: ProxyToolSpec[] = [
  {
    name: "graph.read",
    label: "Knowledge graph read",
    description:
      "Read the knowledge graph with a scoped query. The daemon runs the query " +
      "bounded by this session's read scope and project anchor; pi never touches " +
      "the graph directly.",
  },
  {
    name: "graph.write",
    label: "Knowledge graph write",
    description:
      "Propose a knowledge-graph write. The daemon validates, gates, audits, and " +
      "(when permitted) applies it, registering an undo; pi never writes directly.",
  },
  {
    name: "graph.assert_edge",
    label: "Knowledge graph: add a relationship",
    description:
      "Add a relationship (edge) between two knowledge-graph nodes. Reversible: " +
      "the daemon registers an undo that retracts it. The daemon gates and audits " +
      "the write; pi never touches the graph directly.",
  },
  {
    name: "graph.retract_edge",
    label: "Knowledge graph: remove a relationship",
    description:
      "Retract a relationship (edge) the assistant previously added. Reversible: " +
      "the daemon can re-assert it. The daemon gates and audits the write; pi never " +
      "touches the graph directly.",
  },
];

/** Build the proxy-tools extension factory (`(pi) => void`), registering each
 *  privileged tool as a daemon-forwarding pi custom tool. */
export function makeProxyTools(opts: ProxyOptions = {}): (pi: ProxyExtensionAPI) => void {
  const connect = opts.connect ?? (() => ContractClient.connect());
  const specs = opts.tools ?? DEFAULT_PROXY_TOOLS;
  // One client per engine run, connected on first use; reset on any failure so
  // the next tool call retries the connection.
  let clientPromise: Promise<CallClient> | undefined;

  const fail = (text: string): ProxyToolResult => ({ content: [{ type: "text", text }], isError: true });

  return (pi: ProxyExtensionAPI) => {
    for (const spec of specs) {
      pi.registerTool({
        name: spec.name,
        label: spec.label,
        description: spec.description,
        parameters: PERMISSIVE_PARAMETERS,
        async execute(_toolCallId, params): Promise<ProxyToolResult> {
          let reply: Reply;
          try {
            if (!clientPromise) clientPromise = connect();
            const client = await clientPromise;
            reply = await client.call(calls.execute(spec.name, params));
          } catch (err) {
            clientPromise = undefined;
            return fail(`arlen: ${spec.name} is unavailable (${String(err)})`);
          }

          if (reply.reply !== "execute") {
            return fail(`arlen: unexpected daemon reply '${reply.reply}' for ${spec.name}`);
          }
          if (reply.outcome === "error") {
            // The daemon refused or could not run it (today: the fail-closed
            // runner). Surface it as a tool error, never a silent success.
            return fail(`arlen: ${spec.name} refused (${reply.code}): ${reply.message}`);
          }
          // The daemon ran the action in trusted Rust; surface its result.
          return { content: [{ type: "text", text: JSON.stringify(reply.result) }] };
        },
      });
    }
  };
}

/** The production proxy-tools factory (the default privileged tool set). */
export default makeProxyTools();
