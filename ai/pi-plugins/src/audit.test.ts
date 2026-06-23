import { test } from "node:test";
import assert from "node:assert/strict";
import {
  makeAudit,
  type AuditExtensionAPI,
  type CallClient,
  type ContentBlock,
  type ToolResultEvent,
  type ToolResultEventResult,
} from "./audit.js";
import type { Call, Reply } from "./contract.js";

function fakePi(): { api: AuditExtensionAPI; fire: (event: ToolResultEvent) => Promise<ToolResultEventResult> } {
  let handler: ((event: ToolResultEvent) => Promise<ToolResultEventResult | void> | ToolResultEventResult | void) | undefined;
  const api: AuditExtensionAPI = {
    on(_event, h) {
      handler = h;
    },
  };
  return {
    api,
    async fire(event) {
      assert.ok(handler, "the audit shim registered a tool_result handler");
      return (await handler(event)) ?? {};
    },
  };
}

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

const content: ContentBlock[] = [{ type: "text", text: "rows: a, b, c" }];
function event(): ToolResultEvent {
  return { type: "tool_result", toolCallId: "c7", toolName: "graph.read", input: { query: "x" }, content, isError: false };
}

function isWithheld(r: ToolResultEventResult): boolean {
  return r.isError === true && (r.content?.[0]?.text ?? "").startsWith("[withheld by Arlen");
}

test("a Clean verdict lets the content through and reports the result", async () => {
  const mock = mockClient({ reply: "report", screen: "clean" });
  const pi = fakePi();
  makeAudit({ connect: async () => mock.client })(pi.api);

  const result = await pi.fire(event());
  assert.deepEqual(result, {}); // content unchanged
  assert.equal(mock.calls.length, 1);
  assert.deepEqual(mock.calls[0], {
    call: "report",
    tool_name: "graph.read",
    tool_call_id: "c7",
    result: content,
    is_error: false,
  });
});

test("a Warn verdict also lets the content through", async () => {
  const mock = mockClient({ reply: "report", screen: "warn" });
  const pi = fakePi();
  makeAudit({ connect: async () => mock.client })(pi.api);
  assert.deepEqual(await pi.fire(event()), {});
});

test("a Block verdict withholds the content from the model", async () => {
  const mock = mockClient({ reply: "report", screen: "block" });
  const pi = fakePi();
  makeAudit({ connect: async () => mock.client })(pi.api);
  assert.ok(isWithheld(await pi.fire(event())));
});

test("an unexpected reply withholds fail-closed", async () => {
  const mock = mockClient({ reply: "error", code: "internal" });
  const pi = fakePi();
  makeAudit({ connect: async () => mock.client })(pi.api);
  assert.ok(isWithheld(await pi.fire(event())));
});

test("an unreachable daemon withholds fail-closed and allows a retry", async () => {
  let attempts = 0;
  const pi = fakePi();
  makeAudit({
    connect: async () => {
      attempts += 1;
      throw new Error("ECONNREFUSED");
    },
  })(pi.api);
  assert.ok(isWithheld(await pi.fire(event())));
  await pi.fire(event());
  assert.equal(attempts, 2);
});
