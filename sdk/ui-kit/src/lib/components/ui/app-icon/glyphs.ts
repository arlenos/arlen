// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// The sixteen app glyphs as data: the fifteen launcher apps and the file
/// picker, keyed by the app's id as its desktop file names it without the
/// `arlen-` prefix. Each glyph is a list of SVG elements on Lucide's 24 unit
/// grid, one element per stroke, in Lucide's order, so a gesture can pick the
/// part it moves by index (the folder's lid, the slider's knob, the clock's
/// hand). The drawing is `currentColor`, stroke 2, round caps and joins, no
/// fill; the component supplies those on the enclosing group.
///
/// The path data is Lucide's (ISC; copyright for portions held by Cole Bemis
/// as part of Feather, MIT; all other copyright Lucide Contributors), named
/// per glyph so a drawing can be swapped one at a time.

/// The apps that carry a glyph of their own. The name is the desktop file's
/// `Icon=arlen-<id>` without the prefix, so the shell can look one up from
/// the id it already has.
export type AppId =
  | "files"
  | "settings"
  | "terminal"
  | "system-monitor"
  | "store"
  | "mail"
  | "calendar"
  | "clock"
  | "meetings"
  | "text-editor"
  | "viewers"
  | "pdf"
  | "screenshot"
  | "knowledge"
  | "harness"
  | "file-picker";

/// The apps, in the launcher's order.
export const APP_IDS: readonly AppId[] = [
  "files",
  "settings",
  "terminal",
  "system-monitor",
  "store",
  "mail",
  "calendar",
  "clock",
  "meetings",
  "text-editor",
  "viewers",
  "pdf",
  "screenshot",
  "knowledge",
  "harness",
  "file-picker",
];

/// One element per stroke, on the 24 grid.
export const GLYPHS: Record<AppId, readonly string[]> = {
  // lucide: folder
  files: [
    '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />',
  ],
  // lucide: sliders-horizontal. Three rows: a track on each side of a knob.
  settings: [
    '<path d="M10 5H3" />',
    '<path d="M12 19H3" />',
    '<path d="M14 3v4" />',
    '<path d="M16 17v4" />',
    '<path d="M21 12h-9" />',
    '<path d="M21 19h-5" />',
    '<path d="M21 5h-7" />',
    '<path d="M8 10v4" />',
    '<path d="M8 12H3" />',
  ],
  // lucide: square-terminal
  terminal: [
    '<path d="m7 11 2-2-2-2" />',
    '<path d="M11 13h4" />',
    '<rect width="18" height="18" x="3" y="3" rx="2" ry="2" />',
  ],
  // lucide: activity
  "system-monitor": [
    '<path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2" />',
  ],
  // lucide: package
  store: [
    '<path d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z" />',
    '<path d="M12 22V12" />',
    '<polyline points="3.29 7 12 12 20.71 7" />',
    '<path d="m7.5 4.27 9 5.15" />',
  ],
  // lucide: mail
  mail: [
    '<path d="m22 7-8.991 5.727a2 2 0 0 1-2.009 0L2 7" />',
    '<rect x="2" y="4" width="20" height="16" rx="2" />',
  ],
  // lucide: calendar
  calendar: [
    '<path d="M8 2v4" />',
    '<path d="M16 2v4" />',
    '<rect width="18" height="18" x="3" y="4" rx="2" />',
    '<path d="M3 10h18" />',
  ],
  // lucide: clock
  clock: ['<circle cx="12" cy="12" r="10" />', '<path d="M12 6v6l4 2" />'],
  // lucide: mic
  meetings: [
    '<path d="M12 19v3" />',
    '<path d="M19 10v2a7 7 0 0 1-14 0v-2" />',
    '<rect x="9" y="2" width="6" height="13" rx="3" />',
  ],
  // lucide: pen-line
  "text-editor": [
    '<path d="M13 21h8" />',
    '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" />',
  ],
  // lucide: image
  viewers: [
    '<rect width="18" height="18" x="3" y="3" rx="2" ry="2" />',
    '<circle cx="9" cy="9" r="2" />',
    '<path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21" />',
  ],
  // lucide: file-text
  pdf: [
    '<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z" />',
    '<path d="M14 2v5a1 1 0 0 0 1 1h5" />',
    '<path d="M10 9H8" />',
    '<path d="M16 13H8" />',
    '<path d="M16 17H8" />',
  ],
  // lucide: camera
  screenshot: [
    '<path d="M13.997 4a2 2 0 0 1 1.76 1.05l.486.9A2 2 0 0 0 18.003 7H20a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2h1.997a2 2 0 0 0 1.759-1.048l.489-.904A2 2 0 0 1 10.004 4z" />',
    '<circle cx="12" cy="13" r="3" />',
  ],
  // lucide: brain
  knowledge: [
    '<path d="M12 18V5" />',
    '<path d="M15 13a4.17 4.17 0 0 1-3-4 4.17 4.17 0 0 1-3 4" />',
    '<path d="M17.598 6.5A3 3 0 1 0 12 5a3 3 0 1 0-5.598 1.5" />',
    '<path d="M17.997 5.125a4 4 0 0 1 2.526 5.77" />',
    '<path d="M18 18a4 4 0 0 0 2-7.464" />',
    '<path d="M19.967 17.483A4 4 0 1 1 12 18a4 4 0 1 1-7.967-.517" />',
    '<path d="M6 18a4 4 0 0 1-2-7.464" />',
    '<path d="M6.003 5.125a4 4 0 0 0-2.526 5.77" />',
  ],
  // lucide: sparkles
  harness: [
    '<path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z" />',
    '<path d="M20 2v4" />',
    '<path d="M22 4h-4" />',
    '<circle cx="4" cy="20" r="2" />',
  ],
  // lucide: folder-search
  "file-picker": [
    '<path d="M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1" />',
    '<path d="m21 21-1.9-1.9" />',
    '<circle cx="17" cy="17" r="3" />',
  ],
};

/// The states an icon can be in. `rest` is the drawing; `hover` lifts the
/// plate and moves one thing in the glyph, and is undone when the state goes
/// back; `open` presses the plate and redraws the glyph once; `notify` rocks
/// it once; `busy` breathes until the state changes.
export type AppIconState = "rest" | "hover" | "open" | "notify" | "busy";

/// The states, for a harness that walks them.
export const APP_ICON_STATES: readonly AppIconState[] = ["rest", "hover", "open", "notify", "busy"];
