/// The user-language layer over wire data: the single place that turns
/// internal identifiers (audit kinds, tool names, capability flags) into the
/// plain sentences the UI shows. Raw values stay available everywhere via
/// expandable details and tooltips; this layer only decides the surface.
///
/// The copy itself lives in the message catalog; these functions take the
/// bound translator (`t`) so the sentences follow the locale. Category labels
/// travel as keys the consumer resolves.
///
/// Copy law (binding): no em-dashes, no middot separators, no internal
/// vocabulary. Short active sentences. Success is silent, only failure is
/// marked.
import type { ActivityEntry, Tone } from "$lib/ledger";
import type { Capability } from "$lib/capability";
import type { Translate } from "@arlen/ui-kit/i18n";

/// A user-facing activity category: the badge and the type filter share it.
export interface Category {
  /// Stable key, used as the filter value.
  key: string;
  /// The i18n key for the badge/filter label, or null for an unknown kind
  /// (the consumer then shows the raw `key`, so nothing is mislabeled).
  labelKey: string | null;
  tone: Tone;
}

const CATEGORIES: Record<string, Category> = {
  change: { key: "change", labelKey: "h.disp.cat.change", tone: "ok" },
  lookup: { key: "lookup", labelKey: "h.disp.cat.lookup", tone: "neutral" },
  question: { key: "question", labelKey: "h.disp.cat.question", tone: "neutral" },
  internet: { key: "internet", labelKey: "h.disp.cat.internet", tone: "info" },
  blocked: { key: "blocked", labelKey: "h.disp.cat.blocked", tone: "warn" },
};

/// Map an audit kind onto its user category. Unknown kinds fall back to a
/// neutral badge carrying the raw kind (labelKey null), so nothing is silently
/// mislabeled.
export function categorize(kind: string): Category {
  switch (kind) {
    case "confirm":
      return CATEGORIES.change;
    case "tool-call":
    case "graph-access":
    case "permission":
      return CATEGORIES.lookup;
    case "query":
      return CATEGORIES.question;
    case "network-call":
      return CATEGORIES.internet;
    case "policy-violation":
      return CATEGORIES.blocked;
    default:
      return { key: kind, labelKey: null, tone: "neutral" };
  }
}

/// The categories offered by the type filter, in display order.
export const FILTER_CATEGORIES: Category[] = [
  CATEGORIES.change,
  CATEGORIES.lookup,
  CATEGORIES.question,
  CATEGORIES.internet,
  CATEGORIES.blocked,
];

/// Known tool ids mapped to the i18n key for their human label. Unknown tools
/// get an honest generic label; the raw name always shows in the details.
const TOOL_LABELS: Record<string, string> = {
  "knowledge/query_graph": "h.disp.tool.queryGraph",
  "files/stat": "h.disp.tool.stat",
  "files/list": "h.disp.tool.list",
};

/// One human line for a tool id like "knowledge/query_graph".
export function toolLabel(tool: string, t: Translate): string {
  return t(TOOL_LABELS[tool] ?? "h.disp.usedTool");
}

/// The auto-tag subject shape the agent writes ("auto-tag FILE_PART_OF on
/// <path>"); turned into a plain sentence with the file name.
const AUTO_TAG = /^auto-tag FILE_PART_OF on (.+)$/;

/// One human sentence for an activity entry. The raw subject stays available
/// in the entry details; this is only the surface line.
export function entrySentence(e: ActivityEntry, t: Translate): string {
  switch (e.kind) {
    case "confirm": {
      const m = AUTO_TAG.exec(e.subject);
      if (m) {
        const name = m[1].split("/").pop() || m[1];
        return e.projectId
          ? t("h.disp.addedToProject", { name, project: e.projectId })
          : t("h.disp.addedToAProject", { name });
      }
      return e.subject || t("h.disp.madeChange");
    }
    case "query":
      return t("h.disp.answeredChat");
    case "tool-call":
      return toolLabel(e.subject, t);
    case "graph-access":
      return t("h.disp.lookedAtRecords");
    case "permission":
      return t("h.disp.checkedAllowed");
    case "network-call":
      return t("h.disp.contactedProvider");
    case "policy-violation":
      return t("h.disp.policyStopped");
    default:
      return e.subject || e.kind;
  }
}

/// Whether an outcome should carry a failure marker. Success is silent;
/// `denied` is already expressed by the Blocked category on policy rows, so
/// only a denial on a non-blocked kind and real errors surface.
export function failureMarker(e: ActivityEntry, t: Translate): string | null {
  if (e.outcome === "error") return t("h.disp.failed");
  if (e.outcome === "denied" && e.kind !== "policy-violation") return t("h.disp.blocked");
  return null;
}

/// Whether an entry is a change the user may undo (the compensate trigger).
export function undoable(e: ActivityEntry): boolean {
  return e.kind === "confirm" && e.outcome !== "error";
}

/// The known read levels mapped to the i18n key for their sentence; unknown
/// levels omit the clause rather than inventing one.
const TIER_SENTENCES: Record<string, string> = {
  none: "h.disp.tier.none",
  metadata: "h.disp.tier.metadata",
  structural: "h.disp.tier.metadata",
  content: "h.disp.tier.content",
  full: "h.disp.tier.content",
};

/// The one quiet status sentence under the composer: capability + posture in
/// plain words. `null` capability means the read failed (unreachable), which
/// the caller renders separately.
export function statusSentence(c: Capability, t: Translate): string {
  if (!c.enabled) return t("h.disp.aiOff");
  const parts = [t("h.disp.aiOn")];
  const tierKey = TIER_SENTENCES[c.tier?.toLowerCase?.() ?? ""];
  if (tierKey) parts.push(t(tierKey));
  parts.push(c.executorLive ? t("h.disp.canUndo") : t("h.disp.suggests"));
  return parts.join(" ");
}

/// Tooltip behind the status line: the technical facts, honestly, in one place.
export function statusTooltip(c: Capability, t: Translate): string {
  const model = [c.provider, c.model].filter(Boolean).join(" ");
  const lines = [];
  if (model) lines.push(t("h.disp.model", { model }));
  lines.push(t("h.disp.noMemory"));
  return lines.join("\n");
}
