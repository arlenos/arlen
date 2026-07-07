import { test } from "node:test";
import assert from "node:assert/strict";
import installArlenShims, { type ArlenExtensionAPI } from "./extension.js";

test("the Arlen extension installs the gate, audit, and proxy-tool hooks", () => {
  const events: string[] = [];
  const tools: string[] = [];
  const pi: ArlenExtensionAPI = {
    on(event: string, _handler: unknown) {
      events.push(event);
    },
    registerTool(def: { name: string }) {
      tools.push(def.name);
    },
  } as ArlenExtensionAPI;

  installArlenShims(pi);

  // The security shims register on their events: the gate on tool_call, the audit
  // on tool_result.
  assert.ok(events.includes("tool_call"), "gate registered on tool_call");
  assert.ok(events.includes("tool_result"), "audit registered on tool_result");
  assert.equal(events.length, 2, "exactly the two shim event hooks");
  // The privileged proxy tools are registered as model-callable tools.
  assert.ok(tools.includes("graph.read"), "graph.read proxy tool registered");
  assert.ok(tools.includes("graph.write"), "graph.write proxy tool registered");
});
