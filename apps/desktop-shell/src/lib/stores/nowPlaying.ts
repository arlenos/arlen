/// The "Now Playing" state the MPRIS applet renders: the active player's track
/// + transport capabilities, plus the other registered players for the switcher.
/// The shell is the universal MPRIS consumer - any app that plays media is a
/// producer here. null = no player registered, so the applet hides entirely.
///
/// The MPRIS D-Bus client (discovery, the active-player ranking, transport
/// calls, position interpolation + `Seeked` resync, art handling) is a coder
/// seam; until it lands the applet drives this mocked state, and the transport
/// actions update it optimistically the way the live path will.

import { get, writable, derived } from "svelte/store";

/// A registered media player (for the switcher row).
export interface MprisPlayer {
  id: string;
  /// The app name (from `Identity` / `DesktopEntry`).
  app: string;
  /// The app icon data-URI, or null (→ a note glyph).
  icon: string | null;
  status: "playing" | "paused" | "stopped";
}

/// The active player's track + transport state.
export interface NowPlaying {
  title: string;
  artist: string;
  album: string;
  /// The album-art URL (`file://` loads directly; remote art is the coder's
  /// egress decision), or null → the app-icon / note-glyph fallback.
  artUrl: string | null;
  status: "playing" | "paused" | "stopped";
  /// Elapsed + total, in seconds (MPRIS is µs; the client converts).
  position: number;
  length: number;
  canSeek: boolean;
  canPrev: boolean;
  canNext: boolean;
  canPause: boolean;
  /// Transport is read-only when false.
  canControl: boolean;
  /// All registered players, including the active one.
  players: MprisPlayer[];
  /// The active (or pinned) player's id.
  activeId: string;
}

/// The current now-playing state. null hides the applet.
export const nowPlaying = writable<NowPlaying | null>(null);

/// The other players (for the switcher), excluding the active one.
export const otherPlayers = derived(nowPlaying, ($n) =>
  $n ? $n.players.filter((p) => p.id !== $n.activeId) : [],
);

/// Toggle play/pause on the active player (optimistic; the client also calls
/// MPRIS `PlayPause`).
export function playPause(): void {
  const n = get(nowPlaying);
  if (!n || !n.canControl) return;
  nowPlaying.set({ ...n, status: n.status === "playing" ? "paused" : "playing" });
  // invoke("mpris_play_pause", { id: n.activeId }) - coder seam.
}

/// Skip to the previous / next track.
export function previous(): void {
  // invoke("mpris_previous", { id }) - coder seam.
}
export function next(): void {
  // invoke("mpris_next", { id }) - coder seam.
}

/// Seek the active player to `seconds` (optimistic; the client calls
/// `SetPosition`).
export function seek(seconds: number): void {
  const n = get(nowPlaying);
  if (!n || !n.canSeek) return;
  nowPlaying.set({ ...n, position: Math.max(0, Math.min(n.length, seconds)) });
  // invoke("mpris_set_position", { id, seconds }) - coder seam.
}

/// Promote + pin a player as the active one (manual override of auto-follow).
export function pinPlayer(id: string): void {
  const n = get(nowPlaying);
  if (!n) return;
  nowPlaying.set({ ...n, activeId: id });
  // invoke("mpris_pin", { id }) - coder seam.
}
