// End-to-end harness driver (pi-agent-adoption.md §C): connect the REAL gate
// plugin to a live daemon contract socket, fire one synthetic tool_call, and
// print the enforced result as JSON. The ai-engine-daemon integration test
// spawns this against a real CapabilityGate dispatcher, so the cross-language
// wire (Node ContractClient <-> Rust daemon over an actual socket) and the gate
// enforcement are proven end to end, not just unit-tested with mocks.
//
// Reads ARLEN_AI_ENGINE_SOCKET + ARLEN_AI_ENGINE_TOKEN_FILE. The daemon writes
// the 0600 token only after learning this process's pid (it mints the session
// bound to that pid), so the driver waits for the token to appear before driving
// the call; otherwise the gate would block on a daemon-unavailable error rather
// than exercise the real Authorize path.

import { existsSync, readFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { makeGate, type GateExtensionAPI, type ToolCallEvent, type ToolCallEventResult } from "./gate.js";

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

async function main(): Promise<void> {
  const toolName = process.argv[2] ?? "note.append";
  const externalTriggered = process.argv[3] === "external";
  await waitForToken();

  // Capture the handler the REAL gate registers (real makeGate -> real
  // ContractClient connecting ARLEN_AI_ENGINE_SOCKET with the token file).
  let handler: ((e: ToolCallEvent) => Promise<ToolCallEventResult | void>) | undefined;
  const pi: GateExtensionAPI = {
    on(_event, h) {
      handler = h as (e: ToolCallEvent) => Promise<ToolCallEventResult | void>;
    },
  };
  makeGate({ externalTriggered })(pi);
  if (!handler) throw new Error("the gate registered no tool_call handler");

  const event: ToolCallEvent = {
    type: "tool_call",
    toolCallId: "e2e-1",
    toolName,
    input: { note: "hi" },
  };
  const result = (await handler(event)) ?? {};
  // The integration test parses this line: the enforced result + the (possibly
  // mutated) input, so a Modify decision is observable too.
  process.stdout.write(JSON.stringify({ result, input: event.input }) + "\n");
}

main()
  .then(() => process.exit(0))
  .catch((err) => {
    process.stderr.write(`e2e driver: ${String(err)}\n`);
    process.exit(1);
  });
