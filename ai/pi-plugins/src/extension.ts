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
// The Arlen-specific proxy tools (graph.read/graph.write as model-callable tools
// that forward to the daemon's Execute verb) exist as a MECHANISM in `proxy.ts`
// (`makeProxyTools`, with the Execute round-trip e2e-proven), but are NOT wired in
// here yet: live registration needs a real-pi check that `registerTool` accepts
// the permissive parameter schema (vs requiring pi's TypeBox surface), AND the
// daemon's Execute runners are fail-closed until the cutover, so the model gains
// nothing from them until the live runners land. The gate + audit shims are the
// value now; the proxy live-wiring pairs with that cutover.

import { makeAudit, type AuditExtensionAPI } from "./audit.js";
import { makeGate, type GateExtensionAPI } from "./gate.js";

/** The pi `ExtensionAPI` surface the Arlen extension uses (both shims' hooks). */
export type ArlenExtensionAPI = GateExtensionAPI & AuditExtensionAPI;

/** Install the Arlen security shims (the daemon-backed gate + audit) on `pi`. */
export function installArlenShims(pi: ArlenExtensionAPI): void {
  makeGate()(pi);
  makeAudit()(pi);
}

/** The extension factory pi loads (its default export). */
export default installArlenShims;
