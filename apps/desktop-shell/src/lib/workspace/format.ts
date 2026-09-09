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

/// Card title: the window title, with the app id as fallback.
///
/// It is NOT cut to a character count here. The card is 60px wide and
/// its label ellipses in CSS at the real pixel boundary, which is the
/// only place that knows the font; a 10-character cap is a measurement
/// of English that then gets ellipsed a second time, spending one glyph
/// on an ellipsis nobody sees. The full name is on the button's title
/// and aria-label, so the visible ellipsis costs the reader nothing.
export function cardTitle(title: string, appId: string): string {
  return title.trim() || appId || "";
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
