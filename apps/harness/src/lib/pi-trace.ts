// Fold pi's `--mode rpc` event stream into the A7 tool-call trace (the transparency
// the ai_query swap temporarily dropped). Pure, so the interpretation is
// unit-tested; the conversation store owns the `listen("pi://event")` plumbing that
// feeds these events in and writes the result onto the pending message.

import type { ToolCall, ToolStatus } from "$lib/stores/conversation";

/// A tool call accumulated from pi's stream, carrying pi's `toolCallId` so a later
/// `tool_execution_end` completes the right entry.
export interface TracedCall {
  id: string;
  call: ToolCall;
}

/// Fold one pi rpc event into the running tool-call trace. Recognises
/// `tool_execution_start` (a new running call) and `tool_execution_end` (completes
/// it by id); every other event passes the trace through unchanged, so pi's richer
/// stream is forward-compatible. NB `tool_call` is NOT a stream event - it is pi's
/// internal `beforeToolCall` gate hook (agent-session.ts), consumed by the Arlen
/// extension, never forwarded to `--mode rpc` stdout; the stream carries only the
/// `tool_execution_*` events, so the card keys off those.
export function applyToolEvent(trace: TracedCall[], event: unknown): TracedCall[] {
  if (!event || typeof event !== "object") return trace;
  const e = event as Record<string, unknown>;

  if (e.type === "tool_execution_start") {
    const id = typeof e.toolCallId === "string" ? e.toolCallId : "";
    const [server, tool] = splitToolName(typeof e.toolName === "string" ? e.toolName : "");
    const call: ToolCall = {
      server,
      tool,
      arguments: stringify(e.args ?? {}),
      result: "",
      status: "running",
    };
    return [...trace, { id, call }];
  }

  if (e.type === "tool_execution_end") {
    const id = typeof e.toolCallId === "string" ? e.toolCallId : "";
    const status: ToolStatus = e.isError === true ? "failed" : "done";
    const result = stringify(e.result ?? "");
    return trace.map((t) =>
      t.id === id ? { ...t, call: { ...t.call, result, status } } : t,
    );
  }

  return trace;
}

/// The plain [`ToolCall`] list for the message model (drops the tracking id).
export function toolCallsOf(trace: TracedCall[]): ToolCall[] {
  return trace.map((t) => t.call);
}

/// The assistant text carried by a pi `message_update` event, for STREAMING the
/// answer into the pending message as it generates (feedback during a slow model
/// turn, instead of a blank wait then the whole answer). pi sends the FULL message
/// so far on each update, so the caller SETS the text, it does not append. Returns
/// `null` for any non-`message_update` event or a non-assistant message, so the
/// caller only streams assistant text. pi's message `content` is a string or an
/// array of typed blocks (`messages.ts`); only the `text` blocks are streamed.
export function assistantTextOf(event: unknown): string | null {
  if (!event || typeof event !== "object") return null;
  const e = event as Record<string, unknown>;
  if (e.type !== "message_update") return null;
  const message = e.message;
  if (!message || typeof message !== "object") return null;
  const m = message as Record<string, unknown>;
  // Only stream the assistant's own text (skip a user/tool echo); a missing role
  // during a turn is the assistant's streaming message, so it is not excluded.
  if (typeof m.role === "string" && m.role !== "assistant") return null;
  const content = m.content;
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .filter(
        (c): c is { type: string; text: string } =>
          !!c &&
          typeof c === "object" &&
          (c as Record<string, unknown>).type === "text" &&
          typeof (c as Record<string, unknown>).text === "string",
      )
      .map((c) => c.text)
      .join("");
  }
  return null;
}

/// Split a pi tool name into `(server, tool)`: the first dotted segment is the
/// server (Arlen proxy tools are namespaced `graph.`/`fs.`/`os.`), the rest the
/// tool. A name with no dot is its own tool under an empty server.
function splitToolName(name: string): [string, string] {
  const dot = name.indexOf(".");
  return dot >= 0 ? [name.slice(0, dot), name.slice(dot + 1)] : ["", name];
}

/// Render a pi value (a string, a content array, or an object) to a plain string
/// for the tool-call card, which shows arguments and results verbatim.
function stringify(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}
