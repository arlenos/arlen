/// A run is the ledger entries of one call chain, in order. A step is in
/// flight until a completion entry for it lands (ai-transparency-surface.md
/// §4c), and that completion fills the step rather than appearing as a second
/// one; an entry with no chain is no run's step.
import { describe, expect, it } from "vitest";
import { groupRuns, running } from "./run";
import type { ActivityEntry } from "./ledger";

function entry(i: number, chain: string | null, subject: string, at: number, durationMs: number | null): ActivityEntry {
  return {
    index: i,
    timestampMicros: at,
    kind: "tool-call",
    actor: "ai-agent",
    subject,
    outcome: "ok",
    nodeTypes: [],
    relations: [],
    resultCount: null,
    durationMs,
    depth: null,
    callChainId: chain,
    projectId: null,
    entryRef: `e${i}`,
  };
}

describe("groupRuns", () => {
  it("groups by chain, newest run first, steps in order", () => {
    const runs = groupRuns([
      entry(0, "a", "files.read", 10, 5),
      entry(1, "b", "graph.search", 30, 7),
      entry(2, "a", "files.diff", 20, 3),
      entry(3, null, "loose", 40, 1),
    ]);
    expect(runs.map((r) => r.chainId)).toEqual(["b", "a"]);
    expect(runs[1].steps.map((s) => s.entry.subject)).toEqual(["files.read", "files.diff"]);
  });

  it("keeps a step in flight until its completion lands, then folds it in", () => {
    const pending = groupRuns([entry(0, "a", "files.diff", 10, null)]);
    expect(running(pending[0])).toBe(true);
    const done = groupRuns([entry(0, "a", "files.diff", 10, null), entry(1, "a", "files.diff", 15, 420)]);
    expect(done[0].steps).toHaveLength(1);
    expect(done[0].steps[0].entry.durationMs).toBe(420);
    expect(running(done[0])).toBe(false);
  });

  it("carries the unmeasured parts as null, not zero", () => {
    const [run] = groupRuns([entry(0, "a", "x", 1, 2)]);
    expect(run.bound).toBeNull();
    expect(run.steps[0].tokens).toBeNull();
    expect(run.steps[0].trace).toBeNull();
  });
});
