/// Tile registry: maps tile-id strings to their Svelte component
/// implementations.
///
/// System-tier tiles ship hardcoded — they're part of the trusted
/// shell binary and don't go through modulesd. Module-tier tiles
/// (Phase 7) get registered here at runtime as the modulesd handshake
/// surfaces them.

import type { Component } from "svelte";

import ProjectTile from "./tiles/ProjectTile.svelte";
import KnowledgeTile from "./tiles/KnowledgeTile.svelte";
import NetworkTile from "./tiles/NetworkTile.svelte";
import BluetoothTile from "./tiles/BluetoothTile.svelte";
import DndTile from "./tiles/DndTile.svelte";
import AirplaneTile from "./tiles/AirplaneTile.svelte";
import BrightnessTile from "./tiles/BrightnessTile.svelte";
import AudioTile from "./tiles/AudioTile.svelte";
import UserRowTile from "./tiles/UserRowTile.svelte";

/// One tile entry the QS panel can render: full id + component.
export interface TileRegistration {
  id: string;
  component: Component;
}

/// Bundled system-tier registrations. Order in this array is the
/// fallback render order when no `quicksettings.toml` exists.
///
/// Theme switching is NOT a tile — it lives in the user-row footer
/// because it's an account/identity-level setting. Row 1 pairs
/// project-context with knowledge-graph (both 1×1 context tiles);
/// rows 2–3 pair the four toggles (Network/Bluetooth/DND/Airplane);
/// rows 4–5 are the full-row sliders. No orphan cells.
export const SYSTEM_TILES: TileRegistration[] = [
  { id: "system.project-context", component: ProjectTile as unknown as Component },
  { id: "system.knowledge", component: KnowledgeTile as unknown as Component },
  { id: "system.network", component: NetworkTile as unknown as Component },
  { id: "system.bluetooth", component: BluetoothTile as unknown as Component },
  { id: "system.dnd", component: DndTile as unknown as Component },
  { id: "system.airplane", component: AirplaneTile as unknown as Component },
  { id: "system.brightness", component: BrightnessTile as unknown as Component },
  { id: "system.audio", component: AudioTile as unknown as Component },
  { id: "system.user-row", component: UserRowTile as unknown as Component },
];

/// Lookup a tile registration by id. Returns `null` for unknown ids
/// so the orchestrator can skip rendering.
export function lookupTile(id: string): TileRegistration | null {
  return SYSTEM_TILES.find((t) => t.id === id) ?? null;
}
