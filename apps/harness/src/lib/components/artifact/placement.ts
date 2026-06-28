/// Where an artifact renders: full inline in the chat, or in the right pane.
/// The rule is kind + size (decided with Tim): visual/glanceable kinds always
/// show inline; text/data kinds show inline when small and move to the pane when
/// large, so the chat stays readable without ever showing a peek/preview. The
/// budgets are starting points, tuned on the rendered result.
import type { Artifact } from "./types";

export type Placement = "inline" | "pane";

const lineCount = (s: string): number => s.split("\n").length;

/// Decide inline vs pane for an artifact.
export function placement(artifact: Artifact): Placement {
  const p = artifact.payload;
  switch (p.kind) {
    // Visual / glanceable: always inline, whatever the size.
    case "chart":
    case "image":
    case "links":
      return "inline";
    // Text / data: inline while small, pane once over budget.
    case "code":
      return lineCount(p.source) > 16 ? "pane" : "inline";
    case "diagram":
      return lineCount(p.source) > 16 ? "pane" : "inline";
    case "table":
      return p.rows.length > 8 || p.columns.length > 6 ? "pane" : "inline";
    case "markdown":
      return lineCount(p.source) > 14 || p.source.length > 800 ? "pane" : "inline";
    default:
      return "inline";
  }
}
