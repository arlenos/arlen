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
import { takeProof } from "./proof-store.js";

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

/** Progressive tool-disclosure threshold (ai-tool-routing.md). At or below this
 *  many proxy tools the catalogue is cheap, so register each tool directly (the
 *  full spec is dumped into pi's prompt). ABOVE it, switch to retrieval-first:
 *  register a single `search_tools` meta-tool + a generic `call_tool` instead, so
 *  the prompt's tool catalogue stays bounded as MCP servers/bridges/company
 *  sources grow the set. A count threshold, not a token count: deterministic, and
 *  the proxy descriptions here are uniformly short. */
export const TOOL_DISCLOSURE_THRESHOLD = 12;

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
  {
    // The sharp edge. The description says plainly that it always asks and that it
    // cannot be undone, because the model chooses whether to reach for this at all:
    // an over-sold tool gets proposed for things a graph read would answer, and each
    // proposal spends a user confirmation. The daemon does not trust the wording -
    // the gate classifies this name Confirm unconditionally, the consent biscuit is
    // minted only after a real user approval and is bound to the exact command and
    // arguments, and the terminal-run server re-verifies it before spawning. Nothing
    // here is a security control; it is honest advertising.
    name: "run_command",
    label: "Run a shell command",
    description:
      "Run a single shell command in a locked-down sandbox (no network, no writes " +
      "outside a scratch directory, only system directories readable) and return " +
      "its output. ALWAYS asks the user first, every time, and CANNOT be undone " +
      "once it runs. Prefer a read-only tool when one would answer the question.",
  },
];

/** Search `specs` for `query`: a case-insensitive substring match over each
 *  tool's name, label and description. An empty query returns all tools. Pure so
 *  the retrieval-first meta-tool is unit-tested without a daemon. */
export function searchTools(specs: ProxyToolSpec[], query: string): ProxyToolSpec[] {
  const q = query.trim().toLowerCase();
  if (q === "") return specs.slice();
  return specs.filter((s) =>
    `${s.name} ${s.label} ${s.description}`.toLowerCase().includes(q),
  );
}

/** Build the proxy-tools extension factory (`(pi) => void`). At or below
 *  [`TOOL_DISCLOSURE_THRESHOLD`] tools it registers each privileged tool directly
 *  (the cheap dump-everything path). Above it, it registers a `search_tools`
 *  meta-tool + a generic `call_tool` instead (retrieval-first), so pi's prompt
 *  tool catalogue stays bounded. Both paths forward to the SAME daemon Execute by
 *  the tool's own name, so the daemon gate/audit/re-validation is identical - the
 *  indirection is prompt-economy only, never a trust change (the daemon, not pi,
 *  is the authority; `call_tool` only ever forwards a name from `specs`). */
export function makeProxyTools(opts: ProxyOptions = {}): (pi: ProxyExtensionAPI) => void {
  const connect = opts.connect ?? (() => ContractClient.connect());
  const specs = opts.tools ?? DEFAULT_PROXY_TOOLS;
  // One client per engine run, connected on first use; reset on any failure so
  // the next tool call retries the connection.
  let clientPromise: Promise<CallClient> | undefined;

  const fail = (text: string): ProxyToolResult => ({ content: [{ type: "text", text }], isError: true });

  // Forward one named privileged call to the daemon: mint/reuse a proof, run
  // Execute, surface the result or a tool error. Shared by the direct tools and
  // the retrieval-first `call_tool`, so the daemon path is byte-identical.
  const forward = async (name: string, params: Record<string, unknown>): Promise<ProxyToolResult> => {
    let reply: Reply;
    try {
      if (!clientPromise) clientPromise = connect();
      const client = await clientPromise;
      // HIGH-1: the daemon's Execute requires a single-use proof a matching
      // Authorize minted. Prefer the proof the gate shim already minted for this
      // call (the live path), avoiding a second Authorize (and a double Confirm).
      // Fall back to self-authorizing when no gate ran for this call (a direct test).
      let proof = takeProof(name, params);
      if (proof === undefined) {
        const auth = await client.call(calls.authorize(name, params, false));
        if (auth.reply !== "authorize") {
          return fail(`arlen: unexpected daemon reply '${auth.reply}' authorizing ${name}`);
        }
        if (auth.decision !== "allow" && auth.decision !== "modify") {
          return fail(`arlen: ${name} was not authorized (${auth.decision})`);
        }
        proof = auth.proof;
      }
      reply = await client.call(calls.execute(name, params, proof));
    } catch (err) {
      clientPromise = undefined;
      return fail(`arlen: ${name} is unavailable (${String(err)})`);
    }

    if (reply.reply !== "execute") {
      return fail(`arlen: unexpected daemon reply '${reply.reply}' for ${name}`);
    }
    if (reply.outcome === "error") {
      // The daemon refused or could not run it (today: the fail-closed runner).
      // Surface it as a tool error, never a silent success.
      return fail(`arlen: ${name} refused (${reply.code}): ${reply.message}`);
    }
    // The daemon ran the action in trusted Rust; surface its result.
    return { content: [{ type: "text", text: JSON.stringify(reply.result) }] };
  };

  return (pi: ProxyExtensionAPI) => {
    if (specs.length <= TOOL_DISCLOSURE_THRESHOLD) {
      for (const spec of specs) {
        pi.registerTool({
          name: spec.name,
          label: spec.label,
          description: spec.description,
          parameters: PERMISSIVE_PARAMETERS,
          execute: (_toolCallId, params) => forward(spec.name, params),
        });
      }
      return;
    }

    // Retrieval-first: only the two meta-tools are dumped; the model searches for
    // a tool then invokes it, so the initial catalogue cost is a constant.
    const byName = new Map(specs.map((s) => [s.name, s]));
    pi.registerTool({
      name: "search_tools",
      label: "Search available tools",
      description:
        "Search the available privileged tools by keyword and return the matching " +
        "tools (name + description). Use this to discover a tool, then invoke it " +
        "with `call_tool`.",
      parameters: {
        type: "object",
        properties: { query: { type: "string" } },
        required: ["query"],
        additionalProperties: false,
      },
      async execute(_toolCallId, params): Promise<ProxyToolResult> {
        const query = typeof params.query === "string" ? params.query : "";
        const found = searchTools(specs, query).map((s) => ({
          name: s.name,
          description: s.description,
        }));
        return { content: [{ type: "text", text: JSON.stringify(found) }] };
      },
    });
    pi.registerTool({
      name: "call_tool",
      label: "Invoke a tool",
      description:
        "Invoke a privileged tool discovered via `search_tools`. `name` is the " +
        "tool's name; `arguments` is its parameter object. The daemon gates, " +
        "validates and audits the call exactly as a direct tool call.",
      parameters: {
        type: "object",
        properties: {
          name: { type: "string" },
          arguments: { type: "object", additionalProperties: true },
        },
        required: ["name"],
        additionalProperties: false,
      },
      async execute(_toolCallId, params): Promise<ProxyToolResult> {
        const name = typeof params.name === "string" ? params.name : "";
        // Only forward a name that is a known proxy tool: the daemon dispatches by
        // name and would refuse an unknown one anyway, but reject it here too so
        // `call_tool` can never be a generic verb-invoker (defense in depth).
        if (!byName.has(name)) {
          return fail(`arlen: call_tool: '${name}' is not an available tool (use search_tools)`);
        }
        const args =
          params.arguments && typeof params.arguments === "object"
            ? (params.arguments as Record<string, unknown>)
            : {};
        return forward(name, args);
      },
    });
  };
}

/** The production proxy-tools factory (the default privileged tool set). */
export default makeProxyTools();
