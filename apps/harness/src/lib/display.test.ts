import { describe, expect, it } from "vitest";
import {
  categorize,
  entrySentence,
  failureMarker,
  statusSentence,
  toolLabel,
  undoable,
} from "./display";
import type { ActivityEntry } from "./ledger";
import type { Capability } from "./capability";

function entry(over: Partial<ActivityEntry>): ActivityEntry {
  return {
    index: 0,
    timestampMicros: 0,
    kind: "query",
    actor: "ai-daemon",
    subject: "",
    outcome: "ok",
    nodeTypes: [],
    relations: [],
    resultCount: null,
    durationMs: null,
    depth: null,
    callChainId: null,
    projectId: null,
    entryRef: "e0",
    ...over,
  };
}

function cap(over: Partial<Capability>): Capability {
  return {
    enabled: true,
    tier: "structural",
    actionMode: "suggest",
    executorLive: false,
    ...over,
  };
}

describe("categorize", () => {
  it("maps wire kinds onto the five user categories", () => {
    expect(categorize("confirm").label).toBe("Change");
    expect(categorize("tool-call").label).toBe("Lookup");
    expect(categorize("graph-access").label).toBe("Lookup");
    expect(categorize("permission").label).toBe("Lookup");
    expect(categorize("query").label).toBe("Question");
    expect(categorize("network-call").label).toBe("Internet");
    expect(categorize("policy-violation").label).toBe("Blocked");
  });

  it("keeps unknown kinds visible rather than mislabeling them", () => {
    const c = categorize("future-kind");
    expect(c.label).toBe("future-kind");
    expect(c.tone).toBe("neutral");
  });
});

describe("entrySentence", () => {
  it("turns the auto-tag subject into a plain sentence with the file name", () => {
    const e = entry({
      kind: "confirm",
      subject: "auto-tag FILE_PART_OF on ~/Documents/thesis/chapters/evaluation.tex",
      projectId: "thesis",
    });
    expect(entrySentence(e)).toBe("Added evaluation.tex to the thesis project");
  });

  it("falls back to the raw subject for unknown change shapes", () => {
    const e = entry({ kind: "confirm", subject: "merged duplicate tags" });
    expect(entrySentence(e)).toBe("merged duplicate tags");
  });

  it("labels known tools and stays honest about unknown ones", () => {
    expect(entrySentence(entry({ kind: "tool-call", subject: "knowledge/query_graph" }))).toBe(
      "Searched your file records",
    );
    expect(entrySentence(entry({ kind: "tool-call", subject: "weird/new_tool" }))).toBe(
      "Used a tool",
    );
    expect(toolLabel("files/stat")).toBe("Checked file details");
  });

  it("keeps unknown kinds readable via subject or kind", () => {
    expect(entrySentence(entry({ kind: "future-kind", subject: "did something" }))).toBe(
      "did something",
    );
    expect(entrySentence(entry({ kind: "future-kind" }))).toBe("future-kind");
  });
});

describe("failureMarker", () => {
  it("is silent on success", () => {
    expect(failureMarker(entry({ outcome: "ok" }))).toBeNull();
  });
  it("marks errors as Failed", () => {
    expect(failureMarker(entry({ outcome: "error" }))).toBe("Failed");
  });
  it("marks denials only where the category does not already say Blocked", () => {
    expect(failureMarker(entry({ kind: "tool-call", outcome: "denied" }))).toBe("Blocked");
    expect(failureMarker(entry({ kind: "policy-violation", outcome: "denied" }))).toBeNull();
  });
});

describe("undoable", () => {
  it("offers undo only on settled changes", () => {
    expect(undoable(entry({ kind: "confirm", outcome: "ok" }))).toBe(true);
    expect(undoable(entry({ kind: "confirm", outcome: "error" }))).toBe(false);
    expect(undoable(entry({ kind: "query" }))).toBe(false);
  });
});

describe("statusSentence", () => {
  it("says off plainly", () => {
    expect(statusSentence(cap({ enabled: false }))).toBe(
      "AI is off. You can turn it on in Settings.",
    );
  });
  it("composes capability and posture without jargon", () => {
    expect(statusSentence(cap({}))).toBe(
      "AI is on. It sees file names and dates, not what is inside files. It only suggests changes.",
    );
    expect(statusSentence(cap({ executorLive: true }))).toContain(
      "It can make small changes you can undo.",
    );
  });
  it("omits the read clause for unknown levels instead of inventing one", () => {
    expect(statusSentence(cap({ tier: "experimental-tier" }))).toBe(
      "AI is on. It only suggests changes.",
    );
  });
});
