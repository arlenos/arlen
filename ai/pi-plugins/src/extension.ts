// The single Arlen pi extension: installs both daemon-backed security shims on
// the pi instance it is loaded into (pi-agent-adoption.md §C). This is the
// default-exported `ExtensionFactory` the pi loader runs; the daemon points pi
// at this file when it spawns the sidecar.
//
// - the GATE (`tool_call` -> Authorize): every tool the model calls, including
//   pi's built-in tools (bash, read, write, ...), is authorized by the daemon
//   before it runs - so the security boundary holds with NO Arlen-specific proxy
//   tools needed.
// - the AUDIT shim (`tool_result` -> Report + screen): every tool result is
//   audited and screened before it re-enters the model's context.
//
// - the PROXY tools (graph.read/graph.write as model-callable tools that forward
//   to the daemon's Execute verb): defined in `proxy.ts` (`makeProxyTools`, Execute
//   round-trip e2e-proven) and now registered here. Each forwards to the daemon's
//   Execute presenting the proof the gate shim minted; the daemon runs the READ
//   over the live `CypherPipeline` (when AI is enabled and a provider is configured,
//   else fail-closed) and the WRITE over `UnixRelationWriter` (when `executor_live`
//   is on, else fail-closed), so the tool is only as capable as the live runners.

import { makeAudit, type AuditExtensionAPI } from "./audit.js";
import { makeGate, type GateExtensionAPI } from "./gate.js";
import { makeProxyTools, type ProxyExtensionAPI } from "./proxy.js";

/** The pi `ExtensionAPI` surface the Arlen extension uses (both shims' hooks). */
export type ArlenExtensionAPI = GateExtensionAPI & AuditExtensionAPI & ProxyExtensionAPI;

/** Install the Arlen security shims (the daemon-backed gate + audit) on `pi`. */
export function installArlenShims(pi: ArlenExtensionAPI): void {
  makeGate()(pi);
  makeAudit()(pi);
  // The privileged proxy tools (graph.read/write, ...) the model may call; each
  // forwards to the daemon's Execute, presenting the proof the gate shim minted.
  makeProxyTools()(pi);
}

/** The extension factory pi loads (its default export). */
export default installArlenShims;
