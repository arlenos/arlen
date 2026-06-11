/// Pure formatting helpers for the workspace indicator family.
/// No state, no IPC — every function maps inputs to display strings
/// or display slices and nothing else, so the strip, the overlay
/// and its columns can share one vocabulary.

import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
import type { WindowInfo } from "$lib/stores/windows.js";

/// Label for a workspace pill in the topbar strip (1-based index).
export function pillLabel(_ws: WorkspaceInfo, i: number): string {
  return String(i + 1);
}

/// Full human label for a workspace: its name, or a positional
/// fallback when unnamed.
export function fullLabel(ws: WorkspaceInfo, i: number): string {
  return ws.name.trim() || `Workspace ${i + 1}`;
}

/// Card title: the window title (app id as fallback), hard-truncated
/// with an ellipsis so a card never wraps.
export function truncateTitle(title: string, appId: string): string {
  const source = title.trim() || appId || "";
  if (source.length <= 10) return source;
  return source.slice(0, 9) + "…";
}

/// Caps the cards shown per workspace column: up to six render
/// directly; beyond that the first five show and the remainder
/// collapses into a "+N" overflow badge.
export function visibleSlice(list: WindowInfo[]): {
  shown: WindowInfo[];
  overflow: number;
} {
  if (list.length <= 6) return { shown: list, overflow: 0 };
  return { shown: list.slice(0, 5), overflow: list.length - 5 };
}
