/// Logical 2-column grid layout engine.
///
/// Resolves the user's `quicksettings.toml` plus the bundled-defaults
/// catalogue into a render-ready ordered tile list. The orchestrator
/// drops tiles whose `available_when` predicate fails and tiles with
/// no registry entry (forward-compat: a future module declares a tile
/// the current shell binary doesn't know about).

import { lookupTile, SYSTEM_TILES, type TileRegistration } from "./registry";

/// Wire-side size enum produced by Rust serde (`TileSize` in
/// `arlen-modules`). One of `"one_by_one"`, `"two_by_one"`,
/// `"two_by_two"`.
export type WireSize = "one_by_one" | "two_by_one" | "two_by_two";

/// Component-prop size string (`"1x1"`, `"2x1"`, `"2x2"`).
export type Size = "1x1" | "2x1" | "2x2";

export interface ResolvedTile {
  id: string;
  component: TileRegistration["component"];
  size: Size;
  /// `true` if the tile spans both columns (any row-wide tile).
  fullRow: boolean;
}

export interface LayoutEntry {
  id: string;
  visible: boolean;
  size: Size;
}

/// Convert the wire `TileSize` enum string to the Size alias the
/// component prop accepts. Unknown values fall through to "1x1".
function sizeFromWire(s: WireSize | string | undefined): Size {
  if (s === "two_by_two") return "2x2";
  if (s === "two_by_one") return "2x1";
  return "1x1";
}

/// Merge user layout entries with bundled defaults.
///
/// - Tiles in the user file keep their order, visibility, and size.
/// - System tiles missing from the user file are appended in their
///   bundled order with default size and `visible = true`.
/// - Tiles in the user file that have no registry entry are dropped
///   (forward-compat).
export function resolveLayout(userEntries: LayoutEntry[]): ResolvedTile[] {
  const seen = new Set<string>();
  const out: ResolvedTile[] = [];

  for (const entry of userEntries) {
    if (seen.has(entry.id)) continue;
    seen.add(entry.id);
    if (!entry.visible) continue;
    const reg = lookupTile(entry.id);
    if (!reg) continue;
    out.push({
      id: entry.id,
      component: reg.component,
      size: entry.size,
      fullRow: entry.size !== "1x1",
    });
  }

  // Append untouched system tiles using their bundled size.
  for (const reg of SYSTEM_TILES) {
    if (seen.has(reg.id)) continue;
    out.push({
      id: reg.id,
      component: reg.component,
      size: defaultSizeFor(reg.id),
      fullRow: defaultSizeFor(reg.id) !== "1x1",
    });
  }

  return out;
}

/// Default size for a system tile id. Mirrors the BUNDLED table in
/// the Rust `quicksettings::defaults` module. Project + knowledge
/// are 1×1 (paired in row 1); sliders + user-row stay 2×1.
function defaultSizeFor(id: string): Size {
  switch (id) {
    case "system.brightness":
    case "system.audio":
    case "system.user-row":
      return "2x1";
    default:
      return "1x1";
  }
}

/// Coerce a raw TileSize string from the Tauri wire (snake_case enum
/// repr) into our internal Size alias. Exposed for the orchestrator.
export { sizeFromWire };
