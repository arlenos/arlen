/// Remote sessions (terminal.md §4.12): the terminal is a CONSUMER of the central
/// Connections authority, not a second manager. The Settings "Connections" panel
/// owns the hosts + keys; the terminal owns the trigger (the quick-connect palette)
/// and the live session chrome. This store holds the palette state + the active
/// remote session's chrome.
///
/// Mock-vs-live: fixture-backed. The real connect (spawn an SSH PTY), the saved /
/// recent hosts (from the Connections panel + broker, KG-grouped by project), the
/// enforced scope bar, and the bootstrap status (blocks vs plain) are the coder's
/// seams; this renders + verifies the surface against fixtures until they land.

import { writable } from "svelte/store";

/// A host saved in the Connections panel (the terminal renders, never edits).
export interface SavedHost {
  id: string;
  label: string;
  user: string;
  host: string;
  port: number;
  /// The KG project this host belongs to (grouping), or null.
  project: string | null;
  lastUsed: string | null;
}

/// A recently-connected host tracked in the KG (promote-to-saved graduates it).
export interface RecentHost {
  id: string;
  user: string;
  host: string;
  port: number;
  lastUsed: string;
}

/// Whether the far side bootstrapped Arlen's shell integration: blocks flow, a
/// plain stream (integration unavailable), or still connecting. Never optimistic.
export type Bootstrap = "connecting" | "blocks" | "plain";

/// The active remote session's chrome. Only the scope the broker actually enforces
/// is shown, never a cosmetic label.
export interface RemoteChrome {
  label: string;
  user: string;
  host: string;
  project: string | null;
  /// The enforced reach, e.g. ["shell", "SFTP"].
  reach: string[];
  /// A jump host, if the connection goes via one.
  via: string | null;
  recorded: boolean;
  /// The private key stayed in the broker (never handed to the terminal).
  keyInBroker: boolean;
  bootstrap: Bootstrap;
}

// Fixture hosts (grouped by KG project; identity is the label, monochrome).
const FIXTURE_SAVED: SavedHost[] = [
  { id: "h1", label: "prod-db", user: "deploy", host: "prod-db.atlas.internal", port: 22, project: "Atlas", lastUsed: "2 hours ago" },
  { id: "h2", label: "staging", user: "deploy", host: "staging.atlas.internal", port: 22, project: "Atlas", lastUsed: "yesterday" },
  { id: "h3", label: "build-01", user: "ci", host: "build-01.nebula.internal", port: 22, project: "Nebula", lastUsed: "3 days ago" },
  { id: "h4", label: "vps", user: "tim", host: "203.0.113.9", port: 22, project: null, lastUsed: "last week" },
];
const FIXTURE_RECENT: RecentHost[] = [
  { id: "r1", user: "root", host: "10.0.0.5", port: 22, lastUsed: "1 hour ago" },
];

/// The saved + recent hosts the palette ranks (fixture until the panel bridges).
export const savedHosts = writable<SavedHost[]>(FIXTURE_SAVED);
export const recentHosts = writable<RecentHost[]>(FIXTURE_RECENT);

/// The quick-connect palette open state + query.
export const paletteOpen = writable(false);
export const query = writable("");

/// The active remote session's chrome, or null for a local session. Live: the
/// backend Session carries the remote fields; here it is a fixture set by connect.
export const activeRemote = writable<RemoteChrome | null>(null);

/// Open / close the quick-connect palette (mirrors the history palette).
export function openQuickConnect(): void {
  query.set("");
  paletteOpen.set(true);
}
export function closeQuickConnect(): void {
  paletteOpen.set(false);
}

/// Connect to a saved host. Live: spawns an SSH PTY (the coder's command) and the
/// session becomes remote; here it sets the fixture chrome so the surface renders.
export function connectSaved(h: SavedHost): void {
  closeQuickConnect();
  activeRemote.set({
    label: h.label,
    user: h.user,
    host: h.host,
    project: h.project,
    reach: ["shell", "SFTP"],
    via: h.project === "Atlas" ? "bastion" : null,
    recorded: true,
    keyInBroker: true,
    // A saved host that bootstraps: blocks; the plain VPS shows the honest fallback.
    bootstrap: h.host.includes("203.0.113") ? "plain" : "blocks",
  });
}

/// Connect to a free-typed `user@host` (connect-once). Not saved to the panel; it
/// becomes a Recent node. Live: same SSH spawn.
export function connectAdHoc(target: string): void {
  const at = target.indexOf("@");
  const user = at > 0 ? target.slice(0, at) : "";
  const host = at > 0 ? target.slice(at + 1) : target;
  closeQuickConnect();
  activeRemote.set({
    label: host,
    user: user || "you",
    host,
    project: null,
    reach: ["shell"],
    via: null,
    recorded: true,
    keyInBroker: true,
    bootstrap: "connecting",
  });
}

/// Cut the broker session and revoke the projected capability (revoke-from-here).
/// Live: kills the SSH session + revokes at the broker.
export function revokeFromHere(): void {
  activeRemote.set(null);
}

/// Promote a Recent host to Saved (writes to the Connections panel). Live: a panel
/// write; here it moves the fixture row.
export function promoteToSaved(r: RecentHost): void {
  recentHosts.update((list) => list.filter((x) => x.id !== r.id));
  savedHosts.update((list) => [
    ...list,
    { id: `s-${r.id}`, label: r.host, user: r.user, host: r.host, port: r.port, project: null, lastUsed: r.lastUsed },
  ]);
}
