import { test } from "node:test";
import assert from "node:assert/strict";
import { makeProxyTools, type CallClient, type ProxyExtensionAPI, type ToolDefinition } from "./proxy.js";
import type { Call, Reply } from "./contract.js";

/** Register the proxy tools against a mock contract client and return them by name. */
function collect(connect: () => Promise<CallClient>): Map<string, ToolDefinition> {
  const tools = new Map<string, ToolDefinition>();
  const pi: ProxyExtensionAPI = {
    registerTool(def) {
      tools.set(def.name, def);
    },
  };
  makeProxyTools({ connect })(pi);
  return tools;
}

test("registers the default privileged tools (graph read + write)", () => {
  const tools = collect(async () => ({ call: async (): Promise<Reply> => ({ reply: "ack" }) }));
  assert.ok(tools.has("graph.read"));
  assert.ok(tools.has("graph.write"));
  // The model-facing parameters are a permissive object (the daemon re-validates).
  assert.deepEqual(tools.get("graph.read")!.parameters, { type: "object", additionalProperties: true });
});

test("an Ok execute outcome surfaces the daemon result and forwards the args", async () => {
  let seen: Call | undefined;
  const tools = collect(async () => ({
    call: async (c: Call): Promise<Reply> => {
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
  });
});

test("an error outcome surfaces a tool error (fail-closed, never silent success)", async () => {
  const tools = collect(async () => ({
    call: async (): Promise<Reply> => ({
      reply: "execute",
      outcome: "error",
      code: "permission_denied",
      message: "denied",
    }),
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

test("a daemon-unreachable connect failure is a tool error and retries on the next call", async () => {
  let attempts = 0;
  const connect = async (): Promise<CallClient> => {
    attempts++;
    if (attempts === 1) throw new Error("no socket");
    return { call: async (): Promise<Reply> => ({ reply: "execute", outcome: "ok", result: {} }) };
  };
  const tool = collect(connect).get("graph.read")!;
  const first = await tool.execute("id-1", {});
  assert.equal(first.isError, true);
  const second = await tool.execute("id-2", {});
  assert.equal(second.isError, undefined);
  assert.equal(attempts, 2, "the failed connection is retried on the next call");
});
