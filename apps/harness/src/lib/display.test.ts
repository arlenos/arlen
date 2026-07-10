import { describe, expect, it } from "vitest";
import {
  categorize,
  entrySentence,
  failureMarker,
  statusSentence,
  toolLabel,
  undoable,
} from "./display";
import type { Translate } from "@arlen/ui-kit/i18n";
import type { ActivityEntry } from "./ledger";
import type { Capability } from "./capability";

// A stand-in translator that returns the message id (with any params appended),
// so these unit tests pin the key-mapping + composition LOGIC without depending
// on the catalog copy or the app's build aliases. The English wording itself is
// exercised by the render tests.
const tr: Translate = (id, params) =>
  params ? `${id} ${JSON.stringify(params)}` : id;

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
    expect(categorize("confirm").labelKey).toBe("h.disp.cat.change");
    expect(categorize("tool-call").labelKey).toBe("h.disp.cat.lookup");
    expect(categorize("graph-access").labelKey).toBe("h.disp.cat.lookup");
    expect(categorize("permission").labelKey).toBe("h.disp.cat.lookup");
    expect(categorize("query").labelKey).toBe("h.disp.cat.question");
    expect(categorize("network-call").labelKey).toBe("h.disp.cat.internet");
    expect(categorize("policy-violation").labelKey).toBe("h.disp.cat.blocked");
  });

  it("keeps unknown kinds visible rather than mislabeling them", () => {
    const c = categorize("future-kind");
    expect(c.labelKey).toBeNull();
    expect(c.key).toBe("future-kind");
    expect(c.tone).toBe("neutral");
  });
});

describe("entrySentence", () => {
  it("turns the auto-tag subject into the project message with the file name", () => {
    const e = entry({
      kind: "confirm",
      subject: "auto-tag FILE_PART_OF on ~/Documents/thesis/chapters/evaluation.tex",
      projectId: "thesis",
    });
    const out = entrySentence(e, tr);
    expect(out).toContain("h.disp.addedToProject");
    expect(out).toContain("evaluation.tex");
    expect(out).toContain("thesis");
  });

  it("uses the no-project variant when there is no project", () => {
    const e = entry({ kind: "confirm", subject: "auto-tag FILE_PART_OF on ~/notes/todo.md" });
    const out = entrySentence(e, tr);
    expect(out).toContain("h.disp.addedToAProject");
    expect(out).toContain("todo.md");
  });

  it("falls back to the raw subject for unknown change shapes", () => {
    const e = entry({ kind: "confirm", subject: "merged duplicate tags" });
    expect(entrySentence(e, tr)).toBe("merged duplicate tags");
  });

  it("labels known tools and stays honest about unknown ones", () => {
    expect(entrySentence(entry({ kind: "tool-call", subject: "knowledge/query_graph" }), tr)).toBe(
      "h.disp.tool.queryGraph",
    );
    expect(entrySentence(entry({ kind: "tool-call", subject: "weird/new_tool" }), tr)).toBe(
      "h.disp.usedTool",
    );
    expect(toolLabel("files/stat", tr)).toBe("h.disp.tool.stat");
  });

  it("keeps unknown kinds readable via subject or kind", () => {
    expect(entrySentence(entry({ kind: "future-kind", subject: "did something" }), tr)).toBe(
      "did something",
    );
    expect(entrySentence(entry({ kind: "future-kind" }), tr)).toBe("future-kind");
  });
});

describe("failureMarker", () => {
  it("is silent on success", () => {
    expect(failureMarker(entry({ outcome: "ok" }), tr)).toBeNull();
  });
  it("marks errors as Failed", () => {
    expect(failureMarker(entry({ outcome: "error" }), tr)).toBe("h.disp.failed");
  });
  it("marks denials only where the category does not already say Blocked", () => {
    expect(failureMarker(entry({ kind: "tool-call", outcome: "denied" }), tr)).toBe("h.disp.blocked");
    expect(failureMarker(entry({ kind: "policy-violation", outcome: "denied" }), tr)).toBeNull();
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
    expect(statusSentence(cap({ enabled: false }), tr)).toBe("h.disp.aiOff");
  });
  it("composes on + read level + posture", () => {
    expect(statusSentence(cap({}), tr)).toBe(
      "h.disp.aiOn h.disp.tier.metadata h.disp.suggests",
    );
    expect(statusSentence(cap({ executorLive: true }), tr)).toContain("h.disp.canUndo");
  });
  it("omits the read clause for unknown levels instead of inventing one", () => {
    expect(statusSentence(cap({ tier: "experimental-tier" }), tr)).toBe(
      "h.disp.aiOn h.disp.suggests",
    );
  });
});
