/// The user-language layer over wire data: the single place that turns
/// internal identifiers (audit kinds, tool names, capability flags) into the
/// plain sentences the UI shows. Raw values stay available everywhere via
/// expandable details and tooltips; this layer only decides the surface.
///
/// Copy law (binding): no em-dashes, no middot separators, no internal
/// vocabulary. Short active sentences. Success is silent, only failure is
/// marked.
import type { ActivityEntry, Tone } from "$lib/ledger";
import type { Capability } from "$lib/capability";

/// A user-facing activity category: the badge and the type filter share it.
export interface Category {
  /// Stable key, used as the filter value.
  key: string;
  /// Badge and filter label.
  label: string;
  tone: Tone;
}

const CATEGORIES: Record<string, Category> = {
  change: { key: "change", label: "Change", tone: "ok" },
  lookup: { key: "lookup", label: "Lookup", tone: "neutral" },
  question: { key: "question", label: "Question", tone: "neutral" },
  internet: { key: "internet", label: "Internet", tone: "info" },
  blocked: { key: "blocked", label: "Blocked", tone: "warn" },
};

/// Map an audit kind onto its user category. Unknown kinds fall back to a
/// neutral badge carrying the raw kind, so nothing is silently mislabeled.
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
      return { key: kind, label: kind, tone: "neutral" };
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

/// Human labels for known tools, keyed by the audit subject / tool id the
/// backend reports. Unknown tools get an honest generic label; the raw name
/// always shows in the expanded details.
const TOOL_LABELS: Record<string, string> = {
  "knowledge/query_graph": "Searched your file records",
  "files/stat": "Checked file details",
  "files/list": "Listed a folder",
};

/// One human line for a tool id like "knowledge/query_graph".
export function toolLabel(tool: string): string {
  return TOOL_LABELS[tool] ?? "Used a tool";
}

/// The auto-tag subject shape the agent writes ("auto-tag FILE_PART_OF on
/// <path>"); turned into a plain sentence with the file name.
const AUTO_TAG = /^auto-tag FILE_PART_OF on (.+)$/;

/// One human sentence for an activity entry. The raw subject stays available
/// in the entry details; this is only the surface line.
export function entrySentence(e: ActivityEntry): string {
  switch (e.kind) {
    case "confirm": {
      const m = AUTO_TAG.exec(e.subject);
      if (m) {
        const name = m[1].split("/").pop() || m[1];
        return e.projectId
          ? `Added ${name} to the ${e.projectId} project`
          : `Added ${name} to a project`;
      }
      return e.subject || "Made a change";
    }
    case "query":
      return "Answered a question in Chat";
    case "tool-call":
      return toolLabel(e.subject);
    case "graph-access":
      return "Looked at your file records";
    case "permission":
      return "Checked what it is allowed to do";
    case "network-call":
      return "Contacted the AI provider";
    case "policy-violation":
      return "Tried to go beyond its limits. Stopped";
    default:
      return e.subject || e.kind;
  }
}

/// Whether an outcome should carry a failure marker. Success is silent;
/// `denied` is already expressed by the Blocked category on policy rows, so
/// only a denial on a non-blocked kind and real errors surface.
export function failureMarker(e: ActivityEntry): string | null {
  if (e.outcome === "error") return "Failed";
  if (e.outcome === "denied" && e.kind !== "policy-violation") return "Blocked";
  return null;
}

/// Whether an entry is a change the user may undo (the compensate trigger).
export function undoable(e: ActivityEntry): boolean {
  return e.kind === "confirm" && e.outcome !== "error";
}

/// Sentences for the known read levels; unknown levels omit the clause
/// rather than inventing one.
const TIER_SENTENCES: Record<string, string> = {
  none: "It cannot see your files.",
  metadata: "It sees file names and dates, not what is inside files.",
  structural: "It sees file names and dates, not what is inside files.",
  content: "It can read your files.",
  full: "It can read your files.",
};

/// The one quiet status sentence under the composer: capability + posture in
/// plain words. `null` capability means the read failed (unreachable), which
/// the caller renders separately.
export function statusSentence(c: Capability): string {
  if (!c.enabled) return "AI is off. You can turn it on in Settings.";
  const parts = ["AI is on."];
  const tier = TIER_SENTENCES[c.tier?.toLowerCase?.() ?? ""];
  if (tier) parts.push(tier);
  parts.push(
    c.executorLive
      ? "It can make small changes you can undo."
      : "It only suggests changes.",
  );
  return parts.join(" ");
}

/// The autonomy dial as a short glyph + label for the composer chip: a hollow
/// dial when the agent only suggests, a half dial once it may make reversible
/// changes itself (the `executor_live` gate). Two honest states, not an
/// invented third.
export function tierBadge(c: Capability): { glyph: string; label: string } {
  return c.executorLive
    ? { glyph: "◐", label: "Acts with undo" }
    : { glyph: "○", label: "Suggests only" };
}

/// Tooltip behind the status line: the technical facts, honestly, in one place.
export function statusTooltip(c: Capability): string {
  const model = [c.provider, c.model].filter(Boolean).join(" ");
  const lines = [];
  if (model) lines.push(`Model: ${model}. Change this in Settings.`);
  lines.push("It does not remember earlier questions in this chat yet.");
  return lines.join("\n");
}
