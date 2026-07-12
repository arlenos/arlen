// The Arlen gate plugin: a pi `tool_call` extension that authorizes every tool
// call through the ai-engine-daemon before it runs (pi-agent-adoption.md §C).
//
// pi fires `tool_call` before a tool executes; the handler can BLOCK it (return
// `{ block: true }`) or MODIFY its arguments (mutate `event.input` in place).
// This handler asks the daemon's `Authorize` verb and enforces the verdict
// inline: Allow -> run, Deny -> block, Modify -> substitute args, Confirm ->
// block (no trusted-path consent surface yet, so a Confirm is held, never auto-
// run). The daemon is the authority; the plugin is thin and fail-closed: if the
// daemon is unreachable or answers unexpectedly, the call is BLOCKED.
//
// The pi `ExtensionAPI` is modelled here as the minimal hook surface this plugin
// uses, so the package needs no dependency on the pi package to typecheck; pi
// passes its real (structurally compatible) API at load time.

import { calls, ContractClient, type Reply } from "./contract.js";
import { stashProof } from "./proof-store.js";

/** The one tool whose authorization proof is a per-action consent BISCUIT that
 *  rides in the call's `consent` argument rather than being presented at the
 *  daemon's Execute. `run_command` runs in the SEPARATE terminal-run MCP server,
 *  which the daemon's in-memory proof cannot reach; the daemon mints a
 *  public-key-verifiable token on approval and the MCP server verifies it. Must
 *  match the Rust `RUN_COMMAND_TOOL` in `contracts/run-consent-token`. */
export const RUN_COMMAND_TOOL = "run_command";

/** The subset of pi's `ToolCallEvent` the gate reads. `input` is mutated in
 *  place to apply a Modify decision (pi's documented argument-patch path). */
export interface ToolCallEvent {
  type: "tool_call";
  toolCallId: string;
  toolName: string;
  input: Record<string, unknown>;
}

/** pi's `ToolCallEventResult`: `block` refuses the call, `reason` is shown to
 *  the model. An empty result lets the (possibly mutated) call run. */
export interface ToolCallEventResult {
  block?: boolean;
  reason?: string;
}

/** The minimal pi `ExtensionAPI` surface the gate registers on. */
export interface GateExtensionAPI {
  on(
    event: "tool_call",
    handler: (event: ToolCallEvent, ctx?: unknown) => Promise<ToolCallEventResult | void> | ToolCallEventResult | void,
  ): void;
}

/** Just the `call` method, so a test can inject a mock client. */
export type CallClient = Pick<ContractClient, "call">;

/** Gate configuration (injectable for tests). */
export interface GateOptions {
  /** How to obtain a contract client; defaults to connecting the real socket. */
  connect?: () => Promise<CallClient>;
  /** Whether this run was triggered by external content (escalates the decision
   *  daemon-side). Defaults to false; a richer signal is a later increment. */
  externalTriggered?: boolean;
}

/** Build the gate extension factory. The returned function is what pi loads
 *  (`(pi) => void`); it registers the `tool_call` authorization handler. */
export function makeGate(opts: GateOptions = {}): (pi: GateExtensionAPI) => void {
  const connect = opts.connect ?? (() => ContractClient.connect());
  const externalTriggered = opts.externalTriggered ?? false;
  // One client per engine run, connected on first use. Reset to undefined on any
  // failure so the next tool call retries the connection.
  let clientPromise: Promise<CallClient> | undefined;

  return (pi: GateExtensionAPI) => {
    pi.on("tool_call", async (event): Promise<ToolCallEventResult> => {
      let reply: Reply;
      try {
        if (!clientPromise) clientPromise = connect();
        const client = await clientPromise;
        reply = await client.call(calls.authorize(event.toolName, event.input, externalTriggered));
      } catch (err) {
        clientPromise = undefined; // allow a reconnect on the next call
        return { block: true, reason: `arlen gate: daemon unavailable (${String(err)})` };
      }

      if (reply.reply !== "authorize") {
        return { block: true, reason: `arlen gate: unexpected daemon reply '${reply.reply}'` };
      }
      switch (reply.decision) {
        case "allow":
          if (event.toolName === RUN_COMMAND_TOOL) {
            // run_command runs in the separate terminal-run MCP server, which
            // verifies a per-action consent biscuit at its boundary. The daemon
            // minted that biscuit as the proof; thread it into the call's `consent`
            // argument so it reaches the MCP server. The daemon digested only
            // command+args, so setting `consent` does not change what was authorized.
            // The gate OWNS this field: overwrite any model-supplied value with the
            // daemon's real biscuit (the model must never smuggle a consent token),
            // or DELETE it when no proof was minted (fail-closed: the MCP server then
            // gets no token and refuses). This is the daemon's own argument-patch
            // path (as Modify uses); other tools present the proof at Execute.
            if (reply.proof) event.input.consent = reply.proof;
            else delete event.input.consent;
          } else {
            // Hand the minted proof to the proxy tool that will run (HIGH-1), so it
            // presents it at Execute instead of authorizing a second time.
            stashProof(event.toolName, event.input, reply.proof);
          }
          return {};
        case "deny":
          return { block: true, reason: reply.reason };
        case "modify": {
          // Replace the arguments in place with the daemon's substitution (pi's
          // documented Modify path), then let the (patched) call run.
          if (reply.args && typeof reply.args === "object") {
            for (const k of Object.keys(event.input)) delete event.input[k];
            Object.assign(event.input, reply.args as Record<string, unknown>);
          }
          // The proof binds the SUBSTITUTED args (now in event.input).
          stashProof(event.toolName, event.input, reply.proof);
          return {};
        }
        case "confirm":
          // No trusted-path consent surface yet: hold the call (fail-closed),
          // never auto-run an action the daemon wants confirmed.
          return { block: true, reason: reply.prompt };
        default:
          return { block: true, reason: "arlen gate: unrecognised decision" };
      }
    });
  };
}

/** The production gate factory pi loads as the extension's default export. */
export default makeGate();
