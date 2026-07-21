import { describe, it, expect } from "vitest";
import { applyToolEvent, assistantTextOf, toolCallsOf, type TracedCall } from "./pi-trace";

describe("assistantTextOf", () => {
  it("returns the string content of an assistant message_update", () => {
    expect(
      assistantTextOf({ type: "message_update", message: { role: "assistant", content: "hello" } }),
    ).toBe("hello");
  });

  it("joins the text blocks of an array content, ignoring non-text blocks", () => {
    const ev = {
      type: "message_update",
      message: {
        role: "assistant",
        content: [
          { type: "text", text: "the file " },
          { type: "image", url: "x" },
          { type: "text", text: "is here" },
        ],
      },
    };
    expect(assistantTextOf(ev)).toBe("the file is here");
  });

  it("returns null for non-message_update events and non-assistant messages", () => {
    expect(assistantTextOf({ type: "tool_execution_start", toolCallId: "c1" })).toBeNull();
    expect(assistantTextOf({ type: "message_update", message: { role: "user", content: "hi" } })).toBeNull();
    expect(assistantTextOf({ type: "agent_end" })).toBeNull();
    expect(assistantTextOf(null)).toBeNull();
  });
});

describe("applyToolEvent", () => {
  it("appends a running call for a tool_execution_start event, splitting the namespaced name", () => {
    const trace = applyToolEvent([], {
      type: "tool_execution_start",
      toolName: "graph.query",
      toolCallId: "c1",
      args: { cypher: "MATCH (n) RETURN n" },
    });
    expect(trace).toHaveLength(1);
    expect(trace[0].id).toBe("c1");
    expect(trace[0].call.server).toBe("graph");
    expect(trace[0].call.tool).toBe("query");
    expect(trace[0].call.arguments).toBe('{"cypher":"MATCH (n) RETURN n"}');
    expect(trace[0].call.result).toBe("");
    expect(trace[0].call.status).toBe("running");
  });

  it("completes the matching call on tool_execution_end", () => {
    let trace = applyToolEvent([], { type: "tool_execution_start", toolName: "fs.read", toolCallId: "c2" });
    trace = applyToolEvent(trace, {
      type: "tool_execution_end",
      toolCallId: "c2",
      result: "file contents",
      isError: false,
    });
    expect(trace[0].call.result).toBe("file contents");
    expect(trace[0].call.status).toBe("done");
  });

  it("marks a failed execution", () => {
    let trace = applyToolEvent([], { type: "tool_execution_start", toolName: "os.run", toolCallId: "c3" });
    trace = applyToolEvent(trace, {
      type: "tool_execution_end",
      toolCallId: "c3",
      result: "denied",
      isError: true,
    });
    expect(trace[0].call.status).toBe("failed");
  });

  it("completes only the call whose id matches", () => {
    let trace: TracedCall[] = [];
    trace = applyToolEvent(trace, { type: "tool_execution_start", toolName: "a.x", toolCallId: "c1" });
    trace = applyToolEvent(trace, { type: "tool_execution_start", toolName: "b.y", toolCallId: "c2" });
    trace = applyToolEvent(trace, { type: "tool_execution_end", toolCallId: "c2", result: "ok" });
    expect(trace[0].call.status).toBe("running");
    expect(trace[1].call.status).toBe("done");
  });

  it("splits a name with no dot into an empty server", () => {
    const trace = applyToolEvent([], { type: "tool_execution_start", toolName: "search", toolCallId: "c4" });
    expect(trace[0].call.server).toBe("");
    expect(trace[0].call.tool).toBe("search");
  });

  it("passes unrelated events through unchanged", () => {
    const start: TracedCall[] = [{ id: "c1", call: { server: "g", tool: "q", arguments: "{}", result: "", status: "running" } }];
    expect(applyToolEvent(start, { type: "agent_end" })).toBe(start);
    expect(applyToolEvent(start, null)).toBe(start);
    expect(applyToolEvent(start, "not an object")).toBe(start);
  });

  it("toolCallsOf drops the tracking id", () => {
    const trace: TracedCall[] = [{ id: "c1", call: { server: "g", tool: "q", arguments: "{}", result: "r", status: "done" } }];
    expect(toolCallsOf(trace)).toEqual([{ server: "g", tool: "q", arguments: "{}", result: "r", status: "done" }]);
  });
});
