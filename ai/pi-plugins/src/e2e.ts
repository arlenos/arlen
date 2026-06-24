// End-to-end harness driver (pi-agent-adoption.md §C): connect a REAL Arlen
// plugin to a live daemon contract socket, fire one synthetic pi event, and
// print the enforced result as JSON. The ai-engine-daemon integration test
// spawns this against a real dispatcher, so the cross-language wire (Node
// ContractClient <-> Rust daemon over an actual socket) and the inline
// enforcement are proven end to end, not just unit-tested with mocks.
//
// Modes (argv[2]):
//   gate  <toolName> [external]  - the gate plugin authorizes one tool_call
//   audit <toolName>             - the audit-shim reports one tool_result
//
// Reads ARLEN_AI_ENGINE_SOCKET + ARLEN_AI_ENGINE_TOKEN_FILE. The daemon writes
// the 0600 token only after learning this process's pid (it mints the session
// bound to that pid), so the driver waits for the token before driving the
// event; otherwise a plugin would fail closed on a daemon-unavailable error
// rather than exercise the real verb path.

import { existsSync, readFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { makeGate, type GateExtensionAPI, type ToolCallEvent, type ToolCallEventResult } from "./gate.js";
import {
  makeAudit,
  type AuditExtensionAPI,
  type ToolResultEvent,
  type ToolResultEventResult,
} from "./audit.js";

/** Poll for the session token file the daemon writes once it knows our pid. */
async function waitForToken(): Promise<void> {
  const path = process.env.ARLEN_AI_ENGINE_TOKEN_FILE;
  if (!path) throw new Error("ARLEN_AI_ENGINE_TOKEN_FILE is not set");
  for (let i = 0; i < 200; i++) {
    if (existsSync(path) && readFileSync(path, "utf8").trim().length > 0) return;
    await sleep(25);
  }
  throw new Error("session token file never appeared");
}

/** Drive one tool_call through the real gate plugin, return its enforced result. */
async function driveGate(toolName: string, externalTriggered: boolean): Promise<unknown> {
  let handler: ((e: ToolCallEvent) => Promise<ToolCallEventResult | void>) | undefined;
  const pi: GateExtensionAPI = {
    on(_event, h) {
      handler = h as (e: ToolCallEvent) => Promise<ToolCallEventResult | void>;
    },
  };
  makeGate({ externalTriggered })(pi);
  if (!handler) throw new Error("the gate registered no tool_call handler");
  const event: ToolCallEvent = { type: "tool_call", toolCallId: "e2e-1", toolName, input: { note: "hi" } };
  const result = (await handler(event)) ?? {};
  return { result, input: event.input };
}

/** Drive one tool_result through the real audit-shim, return its enforced result. */
async function driveAudit(toolName: string): Promise<unknown> {
  let handler: ((e: ToolResultEvent) => Promise<ToolResultEventResult | void>) | undefined;
  const pi: AuditExtensionAPI = {
    on(_event, h) {
      handler = h as (e: ToolResultEvent) => Promise<ToolResultEventResult | void>;
    },
  };
  makeAudit()(pi);
  if (!handler) throw new Error("the audit-shim registered no tool_result handler");
  const event: ToolResultEvent = {
    type: "tool_result",
    toolCallId: "e2e-1",
    toolName,
    input: { note: "hi" },
    content: [{ type: "text", text: "tool output" }],
    isError: false,
  };
  const result = (await handler(event)) ?? {};
  return { result };
}

async function main(): Promise<void> {
  const mode = process.argv[2] ?? "gate";
  await waitForToken();

  let out: unknown;
  if (mode === "gate") {
    out = await driveGate(process.argv[3] ?? "note.append", process.argv[4] === "external");
  } else if (mode === "audit") {
    out = await driveAudit(process.argv[3] ?? "graph.read");
  } else {
    throw new Error(`unknown e2e mode: ${mode}`);
  }
  // The integration test parses this line.
  process.stdout.write(JSON.stringify(out) + "\n");
}

main()
  .then(() => process.exit(0))
  .catch((err) => {
    process.stderr.write(`e2e driver: ${String(err)}\n`);
    process.exit(1);
  });
