import { test } from "node:test";
import assert from "node:assert/strict";
import {
  makeProxyTools,
  searchTools,
  TOOL_DISCLOSURE_THRESHOLD,
  type CallClient,
  type ProxyExtensionAPI,
  type ProxyToolSpec,
  type ToolDefinition,
} from "./proxy.js";
import type { Call, Reply } from "./contract.js";

/** Register the proxy tools against a mock contract client and return them by name.
 *  Optional `specs` override the default set (to cross the disclosure threshold). */
function collect(
  connect: () => Promise<CallClient>,
  specs?: ProxyToolSpec[],
): Map<string, ToolDefinition> {
  const tools = new Map<string, ToolDefinition>();
  const pi: ProxyExtensionAPI = {
    registerTool(def) {
      tools.set(def.name, def);
    },
  };
  makeProxyTools({ connect, tools: specs })(pi);
  return tools;
}

/** `n` synthetic proxy tools, enough to cross the disclosure threshold. */
function manySpecs(n: number): ProxyToolSpec[] {
  return Array.from({ length: n }, (_v, i) => ({
    name: `tool.${i}`,
    label: `Tool ${i}`,
    description: i === 0 ? "read the knowledge graph" : `synthetic tool ${i}`,
  }));
}

test("registers the default privileged tools (graph read + write)", () => {
  const tools = collect(async () => ({ call: async (): Promise<Reply> => ({ reply: "ack" }) }));
  assert.ok(tools.has("graph.read"));
  assert.ok(tools.has("graph.write"));
  // graph.read declares its `query` argument so the model provides it (a permissive
  // schema had the model call it with `{}`, which the daemon refuses, looping the
  // turn). The daemon still re-validates; the schema is a model-facing hint only.
  assert.deepEqual(tools.get("graph.read")!.parameters, {
    type: "object",
    properties: {
      query: {
        type: "string",
        description:
          "A natural-language description of what to read from the user's " +
          "knowledge graph. Not Cypher: the daemon generates the query.",
      },
    },
    required: ["query"],
    additionalProperties: false,
  });
  // graph.write keeps the permissive schema (its structured args land with its own
  // executor); only graph.read needed the argument hint for this path.
  assert.deepEqual(tools.get("graph.write")!.parameters, { type: "object", additionalProperties: true });
});

test("an Ok execute outcome surfaces the daemon result and forwards the args", async () => {
  let seen: Call | undefined;
  const tools = collect(async () => ({
    call: async (c: Call): Promise<Reply> => {
      if (c.call === "authorize") return { reply: "authorize", decision: "allow", proof: "test-proof" };
      seen = c;
      return { reply: "execute", outcome: "ok", result: { rows: 1 } };
    },
  }));
  const r = await tools.get("graph.read")!.execute("id-1", { q: "MATCH (n) RETURN n" });
  assert.equal(r.isError, undefined);
  assert.match(r.content[0]?.text ?? "",/rows/);
  assert.deepEqual(seen, {
    call: "execute",
    tool_name: "graph.read",
    tool_input: { q: "MATCH (n) RETURN n" },
    proof: "test-proof",
  });
});

test("an error outcome surfaces a tool error (fail-closed, never silent success)", async () => {
  const tools = collect(async () => ({
    call: async (c: Call): Promise<Reply> =>
      c.call === "authorize"
        ? { reply: "authorize", decision: "allow", proof: "p" }
        : { reply: "execute", outcome: "error", code: "permission_denied", message: "denied" },
  }));
  const r = await tools.get("graph.write")!.execute("id-1", {});
  assert.equal(r.isError, true);
  assert.match(r.content[0]?.text ?? "",/permission_denied|denied|refused/);
});

test("an unexpected daemon reply is a tool error", async () => {
  const tools = collect(async () => ({ call: async (): Promise<Reply> => ({ reply: "ack" }) }));
  const r = await tools.get("graph.read")!.execute("id-1", {});
  assert.equal(r.isError, true);
});

test("a denied authorize blocks the execute (the proxy cannot bypass the gate)", async () => {
  let executed = false;
  const tools = collect(async () => ({
    call: async (c: Call): Promise<Reply> => {
      if (c.call === "authorize") return { reply: "authorize", decision: "deny", reason: "no" };
      executed = true;
      return { reply: "execute", outcome: "ok", result: {} };
    },
  }));
  const r = await tools.get("graph.write")!.execute("id-1", {});
  assert.equal(r.isError, true);
  assert.equal(executed, false, "a denied tool never reaches Execute");
});

test("searchTools matches by keyword over name/label/description; empty query returns all", () => {
  const specs = manySpecs(3);
  assert.deepEqual(
    searchTools(specs, "knowledge").map((s) => s.name),
    ["tool.0"],
    "matches the description of tool.0",
  );
  assert.equal(searchTools(specs, "Tool 2").length, 1, "matches the label");
  assert.equal(searchTools(specs, "").length, 3, "an empty query returns all");
  assert.equal(searchTools(specs, "nonesuch").length, 0);
});

test("a small catalogue dumps each tool directly (no meta-tools)", () => {
  const tools = collect(async () => ({ call: async (): Promise<Reply> => ({ reply: "ack" }) }));
  assert.ok(tools.has("graph.read"));
  assert.equal(tools.has("search_tools"), false);
  assert.equal(tools.has("call_tool"), false);
});

test("a large catalogue switches to retrieval-first (search_tools + call_tool only)", () => {
  const specs = manySpecs(TOOL_DISCLOSURE_THRESHOLD + 1);
  const tools = collect(async () => ({ call: async (): Promise<Reply> => ({ reply: "ack" }) }), specs);
  assert.ok(tools.has("search_tools"), "search_tools is registered above the threshold");
  assert.ok(tools.has("call_tool"), "call_tool is registered above the threshold");
  assert.equal(tools.has("tool.0"), false, "the individual tools are NOT dumped");
  assert.equal(tools.size, 2, "only the two meta-tools are registered");
});

test("search_tools returns the matching tool names + descriptions", async () => {
  const specs = manySpecs(TOOL_DISCLOSURE_THRESHOLD + 1);
  const tools = collect(async () => ({ call: async (): Promise<Reply> => ({ reply: "ack" }) }), specs);
  const r = await tools.get("search_tools")!.execute("id", { query: "knowledge" });
  const found = JSON.parse(r.content[0]?.text ?? "[]");
  assert.deepEqual(found, [{ name: "tool.0", description: "read the knowledge graph" }]);
});

test("call_tool forwards to the daemon by the inner name (same gate path as a direct tool)", async () => {
  let seen: Call | undefined;
  const specs = manySpecs(TOOL_DISCLOSURE_THRESHOLD + 1);
  const tools = collect(async () => ({
    call: async (c: Call): Promise<Reply> => {
      if (c.call === "authorize") return { reply: "authorize", decision: "allow", proof: "p" };
      seen = c;
      return { reply: "execute", outcome: "ok", result: { ok: true } };
    },
  }), specs);
  const r = await tools.get("call_tool")!.execute("id", { name: "tool.5", arguments: { a: 1 } });
  assert.equal(r.isError, undefined);
  assert.deepEqual(seen, { call: "execute", tool_name: "tool.5", tool_input: { a: 1 }, proof: "p" });
});

test("call_tool rejects a name that is not a known tool (never a generic verb-invoker)", async () => {
  let called = false;
  const specs = manySpecs(TOOL_DISCLOSURE_THRESHOLD + 1);
  const tools = collect(async () => ({
    call: async (): Promise<Reply> => {
      called = true;
      return { reply: "ack" };
    },
  }), specs);
  const r = await tools.get("call_tool")!.execute("id", { name: "rm.rf", arguments: {} });
  assert.equal(r.isError, true);
  assert.equal(called, false, "an unknown tool name never reaches the daemon");
});

test("a daemon-unreachable connect failure is a tool error and retries on the next call", async () => {
  let attempts = 0;
  const connect = async (): Promise<CallClient> => {
    attempts++;
    if (attempts === 1) throw new Error("no socket");
    return {
      call: async (c: Call): Promise<Reply> =>
        c.call === "authorize"
          ? { reply: "authorize", decision: "allow", proof: "p" }
          : { reply: "execute", outcome: "ok", result: {} },
    };
  };
  const tool = collect(connect).get("graph.read")!;
  const first = await tool.execute("id-1", {});
  assert.equal(first.isError, true);
  const second = await tool.execute("id-2", {});
  assert.equal(second.isError, undefined);
  assert.equal(attempts, 2, "the failed connection is retried on the next call");
});
