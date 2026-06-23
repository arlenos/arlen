// The Arlen audit-shim plugin: a pi `tool_result` extension that reports every
// tool result to the daemon for audit + S17/S18 screening BEFORE the content
// re-enters the model's context (pi-agent-adoption.md §C).
//
// pi fires `tool_result` after a tool runs; returning `content` REPLACES what
// the model sees. This handler sends a `Report` to the daemon and, on a `Block`
// screen verdict, substitutes a placeholder so the flagged content never reaches
// the model. It is FAIL-CLOSED: a result that could not be screened (daemon
// unreachable, or an unexpected reply) is also withheld - the action-gate
// authorizes the action, but it does not vet the result's CONTENT for injection,
// so unscreened content must not reach the model. A `Clean`/`Warn` verdict lets
// the content through unchanged (Warn is logged daemon-side; the gate's
// confirm-on-external-trigger is the action-level containment).
//
// As in the gate plugin, pi's event/result types are modelled as the minimal
// local hook surface, so the package needs no dependency on the pi package.

import { calls, ContractClient, type Reply } from "./contract.js";

/** A content block in a tool result (text or image; only `text` is read here). */
export interface ContentBlock {
  type: string;
  text?: string;
  [key: string]: unknown;
}

/** The subset of pi's `ToolResultEvent` the shim reads. */
export interface ToolResultEvent {
  type: "tool_result";
  toolCallId: string;
  toolName: string;
  input: Record<string, unknown>;
  content: ContentBlock[];
  isError: boolean;
}

/** pi's `ToolResultEventResult`: returning `content` replaces the model-visible
 *  result; omitting it lets the original through. */
export interface ToolResultEventResult {
  content?: ContentBlock[];
  details?: unknown;
  isError?: boolean;
}

/** The minimal pi `ExtensionAPI` surface the shim registers on. */
export interface AuditExtensionAPI {
  on(
    event: "tool_result",
    handler: (event: ToolResultEvent, ctx?: unknown) => Promise<ToolResultEventResult | void> | ToolResultEventResult | void,
  ): void;
}

/** Just the `call` method, so a test can inject a mock client. */
export type CallClient = Pick<ContractClient, "call">;

/** Audit-shim configuration (injectable for tests). */
export interface AuditOptions {
  /** How to obtain a contract client; defaults to connecting the real socket. */
  connect?: () => Promise<CallClient>;
}

/** The placeholder shown to the model in place of withheld content. */
const WITHHELD: ContentBlock[] = [
  {
    type: "text",
    text: "[withheld by Arlen: the tool result was flagged or could not be screened, and is not shown to the model]",
  },
];

/** Build the audit-shim extension factory (`(pi) => void`), registering the
 *  `tool_result` Report-and-screen handler. */
export function makeAudit(opts: AuditOptions = {}): (pi: AuditExtensionAPI) => void {
  const connect = opts.connect ?? (() => ContractClient.connect());
  let clientPromise: Promise<CallClient> | undefined;

  return (pi: AuditExtensionAPI) => {
    pi.on("tool_result", async (event): Promise<ToolResultEventResult> => {
      let reply: Reply;
      try {
        if (!clientPromise) clientPromise = connect();
        const client = await clientPromise;
        reply = await client.call(calls.report(event.toolName, event.toolCallId, event.content, event.isError));
      } catch (err) {
        clientPromise = undefined; // allow a reconnect on the next result
        // Fail-closed: an unscreened result must not reach the model.
        return { content: WITHHELD, isError: true };
      }

      if (reply.reply !== "report") {
        return { content: WITHHELD, isError: true };
      }
      if (reply.screen === "block") {
        return { content: WITHHELD, isError: true };
      }
      // clean | warn: the content may re-enter the model context unchanged.
      return {};
    });
  };
}

/** The production audit-shim factory pi loads as the extension's default export. */
export default makeAudit();
