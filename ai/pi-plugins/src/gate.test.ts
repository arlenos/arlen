import { test } from "node:test";
import assert from "node:assert/strict";
import { makeGate, type CallClient, type GateExtensionAPI, type ToolCallEvent, type ToolCallEventResult } from "./gate.js";
import type { Call, Reply } from "./contract.js";

/** A fake pi that captures the registered tool_call handler so a test can fire
 *  it directly. */
function fakePi(): {
  api: GateExtensionAPI;
  fire: (event: ToolCallEvent) => Promise<ToolCallEventResult>;
} {
  let handler: ((event: ToolCallEvent) => Promise<ToolCallEventResult | void> | ToolCallEventResult | void) | undefined;
  const api: GateExtensionAPI = {
    on(_event, h) {
      handler = h;
    },
  };
  return {
    api,
    async fire(event) {
      assert.ok(handler, "the gate registered a tool_call handler");
      return (await handler(event)) ?? {};
    },
  };
}

/** A mock client returning a canned reply and recording the calls it received.
 *  (The daemon-unreachable path is exercised by throwing at `connect` instead.) */
function mockClient(reply: Reply): { client: CallClient; calls: Call[] } {
  const calls: Call[] = [];
  const client: CallClient = {
    async call(call: Call): Promise<Reply> {
      calls.push(call);
      return reply;
    },
  };
  return { client, calls };
}

function event(toolName: string, input: Record<string, unknown>): ToolCallEvent {
  return { type: "tool_call", toolCallId: "c1", toolName, input };
}

test("Allow lets the call run (empty result) and authorizes with the tool name + input", async () => {
  const mock = mockClient({ reply: "authorize", decision: "allow" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client, externalTriggered: true })(pi.api);

  const result = await pi.fire(event("graph.read", { query: "x" }));
  assert.deepEqual(result, {});
  assert.equal(mock.calls.length, 1);
  assert.deepEqual(mock.calls[0], {
    call: "authorize",
    tool_name: "graph.read",
    tool_input: { query: "x" },
    external_triggered: true,
  });
});

test("Deny blocks the call with the daemon's reason", async () => {
  const mock = mockClient({ reply: "authorize", decision: "deny", reason: "out of scope" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  assert.deepEqual(await pi.fire(event("graph.write", {})), { block: true, reason: "out of scope" });
});

test("Modify substitutes the arguments in place and lets the call run", async () => {
  const mock = mockClient({ reply: "authorize", decision: "modify", args: { query: "SAFE" } });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  const ev = event("graph.read", { query: "RAW", extra: 1 });
  const result = await pi.fire(ev);
  assert.deepEqual(result, {});
  // The old args are gone; only the daemon's substitution remains.
  assert.deepEqual(ev.input, { query: "SAFE" });
});

test("Confirm holds the call (blocked) with the prompt, never auto-runs", async () => {
  const mock = mockClient({ reply: "authorize", decision: "confirm", prompt: "Delete everything?" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  assert.deepEqual(await pi.fire(event("fs.delete", {})), { block: true, reason: "Delete everything?" });
});

test("Allow on run_command threads the consent biscuit into the call's `consent` arg", async () => {
  const mock = mockClient({ reply: "authorize", decision: "allow", proof: "biscuit-xyz" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  const ev = event("run_command", { command: "ls", args: ["-la"] });
  const result = await pi.fire(ev);
  assert.deepEqual(result, {});
  // The biscuit rides in the call arguments the MCP server verifies; command+args
  // (the digested part) are untouched.
  assert.deepEqual(ev.input, { command: "ls", args: ["-la"], consent: "biscuit-xyz" });
});

test("Allow on run_command overwrites any model-supplied `consent` (no smuggling)", async () => {
  const mock = mockClient({ reply: "authorize", decision: "allow", proof: "real-biscuit" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  // The model tries to smuggle a forged consent token in its own arguments.
  const ev = event("run_command", { command: "ls", consent: "forged-by-model" });
  await pi.fire(ev);
  // The gate owns the field: the daemon's real biscuit replaces the forged one.
  assert.equal(ev.input.consent, "real-biscuit");
});

test("Allow on run_command with no minted proof deletes any `consent` (fail-closed)", async () => {
  const mock = mockClient({ reply: "authorize", decision: "allow" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  // Even a model-supplied consent is removed when the daemon minted no proof.
  const ev = event("run_command", { command: "ls", consent: "forged-by-model" });
  await pi.fire(ev);
  assert.deepEqual(ev.input, { command: "ls" });
});

test("an unexpected reply blocks fail-closed", async () => {
  const mock = mockClient({ reply: "error", code: "internal" });
  const pi = fakePi();
  makeGate({ connect: async () => mock.client })(pi.api);
  const r = await pi.fire(event("graph.read", {}));
  assert.equal(r.block, true);
});

test("an unreachable daemon blocks fail-closed and allows a later retry", async () => {
  let attempts = 0;
  const pi = fakePi();
  makeGate({
    connect: async () => {
      attempts += 1;
      throw new Error("ECONNREFUSED");
    },
  })(pi.api);
  const r = await pi.fire(event("graph.read", {}));
  assert.equal(r.block, true);
  assert.match(r.reason ?? "", /daemon unavailable/);
  // A second call retries the connection (not stuck on the first rejection).
  await pi.fire(event("graph.read", {}));
  assert.equal(attempts, 2);
});
