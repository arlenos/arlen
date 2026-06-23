import { test } from "node:test";
import assert from "node:assert/strict";
import installArlenShims, { type ArlenExtensionAPI } from "./extension.js";

test("the Arlen extension installs both the gate and the audit hooks", () => {
  const events: string[] = [];
  const pi: ArlenExtensionAPI = {
    on(event: string, _handler: unknown) {
      events.push(event);
    },
  } as ArlenExtensionAPI;

  installArlenShims(pi);

  // Both security shims register: the gate on tool_call, the audit on tool_result.
  assert.ok(events.includes("tool_call"), "gate registered on tool_call");
  assert.ok(events.includes("tool_result"), "audit registered on tool_result");
  assert.equal(events.length, 2, "exactly the two shim hooks, nothing else");
});
