/// The run view's model (ai-transparency-surface.md §4b): one run is the
/// ledger entries that share a `call_chain_id`, in order, each a step. A step
/// with no `durationMs` is still in flight (§4c: the before-act entry is
/// written first and a completion entry fills the number), so "how long it
/// has waited" is now minus its start.
///
/// Two parts of §4b have no feed yet and are carried as null rather than
/// guessed: tokens per step (only the session total exists) and the
/// reasoning trace (the daemon's event stream). The loop's bound is null
/// for the same reason. The page says so in each place instead of drawing a
/// zero, which would read as "used nothing".
import type { ActivityEntry } from "./ledger";

export interface RunStep {
  /// The ledger entry this step is.
  entry: ActivityEntry;
  /// Tokens this step consumed, or null when not measured.
  tokens: number | null;
  /// What the model worked through in this step, or null when no trace feed.
  trace: string | null;
}

export interface Run {
  chainId: string;
  steps: RunStep[];
  /// The loop's step bound, or null when the orchestrator does not report it.
  bound: number | null;
}

/// Group a ledger page into runs by `call_chain_id`, newest run first, each
/// run's steps oldest first. Entries without a chain id are not part of a run.
/// A completion entry (same chain, same subject, carrying the duration) is
/// folded into the step it completes rather than drawn as a second step.
export function groupRuns(entries: ActivityEntry[]): Run[] {
  const byChain = new Map<string, ActivityEntry[]>();
  for (const e of entries) {
    if (!e.callChainId) continue;
    const list = byChain.get(e.callChainId) ?? [];
    list.push(e);
    byChain.set(e.callChainId, list);
  }
  const runs: Run[] = [];
  for (const [chainId, list] of byChain) {
    list.sort((a, b) => a.timestampMicros - b.timestampMicros);
    const steps: RunStep[] = [];
    for (const e of list) {
      const open = steps.find((s) => s.entry.durationMs === null && s.entry.subject === e.subject && s.entry.kind === e.kind);
      if (open && e.durationMs !== null) {
        open.entry = { ...open.entry, durationMs: e.durationMs, outcome: e.outcome };
        continue;
      }
      steps.push({ entry: e, tokens: null, trace: null });
    }
    runs.push({ chainId, steps, bound: null });
  }
  runs.sort((a, b) => last(b) - last(a));
  return runs;
}

function last(r: Run): number {
  return r.steps.length ? r.steps[r.steps.length - 1].entry.timestampMicros : 0;
}

/// Whether the run is still going: its last step has not completed.
export function running(r: Run): boolean {
  const s = r.steps[r.steps.length - 1];
  return !!s && s.entry.durationMs === null;
}

/// A sample run for the hostless view, with every §4b part filled so the
/// design can be judged: six steps, one of them ten times its neighbours'
/// tokens, the last one still waiting on a tool.
export function sampleRun(nowMicros: number): Run {
  const base = nowMicros - 41_000_000;
  const step = (
    i: number,
    kind: string,
    subject: string,
    atS: number,
    durationMs: number | null,
    tokens: number,
    trace: string | null,
  ): RunStep => ({
    entry: {
      index: i,
      timestampMicros: base + atS * 1_000_000,
      kind,
      actor: "ai-agent",
      subject,
      outcome: durationMs === null ? "pending" : "ok",
      nodeTypes: [],
      relations: [],
      resultCount: null,
      durationMs,
      depth: null,
      callChainId: "sample",
      projectId: null,
      entryRef: `sample-${i}`,
    },
    tokens,
    trace,
  });
  return {
    chainId: "sample",
    bound: 12,
    steps: [
      step(0, "query", "", 0, 1_900, 820, "The question is about this week's plan. Look up what changed in the project first."),
      step(1, "tool-call", "graph.search", 2, 640, 310, null),
      step(2, "graph-access", "", 3, 220, 140, null),
      step(3, "query", "", 4, 9_800, 8_400, "Three files changed. Read each one and compare against last week's version before summarising."),
      step(4, "tool-call", "files.read", 15, 1_200, 520, null),
      step(5, "tool-call", "files.diff", 17, null, 0, null),
    ],
  };
}
