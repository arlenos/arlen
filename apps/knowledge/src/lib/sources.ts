// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// The name a person knows a source by. The graph records an application by its
/// id (`files`, `dev.arlen.files`, `foot`), and the timeline's live rows showed
/// that id where the sample showed "Files": one column, two vocabularies. This
/// table names the first-party applications in both spellings the graph uses;
/// an id it does not know is shown as it is, because a guessed name would be a
/// claim about a program nobody looked up.
const NAMES: Record<string, string> = {
  files: "Files",
  terminal: "Terminal",
  foot: "Terminal",
  "text-editor": "Text editor",
  editor: "Text editor",
  shell: "Shell",
  "desktop-shell": "Shell",
  harness: "Assistant",
  assistant: "Assistant",
  mail: "Mail",
  calendar: "Calendar",
  store: "Store",
  meetings: "Meetings",
  viewers: "Viewer",
  pdf: "Reader",
  knowledge: "Knowledge",
  clock: "Clock",
  screenshot: "Screenshot",
  settings: "Settings",
  "system-monitor": "Task manager",
};

/// The display name for a source id, or the id itself when none is known.
export function sourceName(id: string): string {
  const bare = id.replace(/^(dev|org)\.arlen\./, "");
  return NAMES[bare] ?? NAMES[id] ?? id;
}
