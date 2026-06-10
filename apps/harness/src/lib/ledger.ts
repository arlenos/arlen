/// Wire types for the agent dashboard's read paths: the audit-ledger
/// activity page (shared S-U4 read command), behaviour status, and anomaly
/// notices. Mirrors the backend payloads as the TS code consumes them.

/// One audit-ledger entry (Structural tier, content-free).
export interface ActivityEntry {
  index: number;
  timestampMicros: number;
  kind: string;
  actor: string;
  subject: string;
  outcome: string;
  nodeTypes: string[];
  relations: string[];
  resultCount: number | null;
  durationMs: number | null;
  depth: number | null;
  callChainId: string | null;
  projectId: string | null;
  entryRef: string;
}

/// One page of ledger entries plus the ledger's own health.
export interface ActivityPage {
  entries: ActivityEntry[];
  available: boolean;
  tampered: boolean;
  total: number;
}

/// One anomaly-detector notice (the rare important warning channel).
export interface Notice {
  kind: string;
  summary: string;
  body: string;
  critical: boolean;
  tsMicros: number;
}

/// The notices read; `available: false` means the alert log was unreadable
/// (degraded source, not all-clear).
export interface NoticesResult {
  available: boolean;
  notices: Notice[];
}

/// Semantic badge tone, mapped to theme tokens by the row renderer.
export type Tone = "neutral" | "ok" | "warn" | "info";

/// Human label + tone per audit kind. Unknown kinds fall back to the raw
/// kind with the neutral tone.
export const KIND_META: Record<string, { label: string; tone: Tone }> = {
  query: { label: "Query", tone: "neutral" },
  "tool-call": { label: "Tool call", tone: "info" },
  confirm: { label: "Confirmed", tone: "ok" },
  "policy-violation": { label: "Blocked", tone: "warn" },
  "graph-access": { label: "Graph", tone: "neutral" },
  permission: { label: "Permission", tone: "info" },
  "network-call": { label: "Network", tone: "info" },
};
