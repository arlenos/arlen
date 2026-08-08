/// The wallpaper picker (wallpaper-plan.md WP-R1): choose the desktop background.
///
/// Mock-vs-live: the wallpaper daemon (`daemons/wallpaper`) renders + schedules;
/// the app-side Tauri bridge (`list_wallpapers` / `set_wallpaper` / `add_wallpaper`)
/// is a coder seam not built yet, so a fixture set stands in under vite. Real
/// wallpapers are image sources; the fixtures use CSS gradients so the grid, the
/// set, and the add flow all render without image files.
import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// How the image maps to the screen (the daemon's `Scale`).
export type WallpaperScale = "fill" | "fit" | "center" | "tile" | "stretch";

/// One choosable wallpaper.
export interface WallpaperEntry {
  id: string;
  /// A human name for the tile.
  name: string;
  /// A CSS background value for the thumbnail. Live: an image url; the fixtures
  /// use gradients so the surface renders without shipped files.
  thumb: string;
  /// Static image today; `live` (video / sandboxed shader) is WP-R2.
  kind: "static" | "live";
}

const FIXTURE: WallpaperEntry[] = [
  { id: "wp-nightfall", name: "Nightfall", thumb: "linear-gradient(135deg, #0f172a, #4c1d95)", kind: "static" },
  { id: "wp-aurora", name: "Aurora", thumb: "linear-gradient(135deg, #064e3b, #0ea5e9)", kind: "static" },
  { id: "wp-ember", name: "Ember", thumb: "linear-gradient(135deg, #431407, #f97316)", kind: "static" },
  { id: "wp-slate", name: "Slate", thumb: "linear-gradient(135deg, #111827, #374151)", kind: "static" },
  { id: "wp-rose", name: "Rose", thumb: "linear-gradient(135deg, #4c0519, #fb7185)", kind: "static" },
  { id: "wp-mono", name: "Mono", thumb: "linear-gradient(135deg, #0a0a0a, #262626)", kind: "static" },
];

/// The available wallpapers.
export const wallpapers = writable<WallpaperEntry[]>([]);

/// True when a real session could not read the installed wallpapers.
export const wallpapersUnavailable = writable(false);
/// The id of the active wallpaper, or null.
/// True when the last wallpaper change did not reach the daemon, so the picker
/// still highlights the one actually on the desktop.
export const wallpaperChangeFailed = writable(false);

export const currentId = writable<string | null>(null);
/// The active fit mode.
export const scale = writable<WallpaperScale>("fill");

let added = 0;

/// Load the wallpaper set + the current selection. Live: `list_wallpapers`.
export async function listWallpapers(): Promise<void> {
  try {
    const list = await invoke<WallpaperEntry[]>("list_wallpapers");
    wallpapers.set(list);
    wallpapersUnavailable.set(false);
  } catch {
    if (import.meta.env.DEV) {
      wallpapers.set(FIXTURE);
      currentId.set("wp-nightfall");
      wallpapersUnavailable.set(false);
      return;
    }
    // This one also asserted which wallpaper is currently set, and every tile
    // calls `set_wallpaper` with its id - so a failed read offered a grid of
    // wallpapers that are not installed, one of them marked as the active one.
    wallpapers.set([]);
    currentId.set(null);
    wallpapersUnavailable.set(true);
  }
}

/// Set the active wallpaper (+ fit). Live: `set_wallpaper`; the store update stands
/// under vite.
export async function setWallpaper(id: string): Promise<void> {
  const before = get(currentId);
  currentId.set(id);
  wallpaperChangeFailed.set(false);
  const s = get(scale);
  try {
    await invoke("set_wallpaper", { id, scale: s });
  } catch {
    if (import.meta.env.DEV) return; // no daemon under vite
    // The desktop still shows the old one, so the picker must too.
    currentId.set(before);
    wallpaperChangeFailed.set(true);
  }
}

/// Change the fit mode and re-apply it to the current wallpaper.
export async function setScale(next: WallpaperScale): Promise<void> {
  const before = get(scale);
  scale.set(next);
  wallpaperChangeFailed.set(false);
  const id = get(currentId);
  if (id === null) return;
  try {
    await invoke("set_wallpaper", { id, scale: next });
  } catch {
    if (import.meta.env.DEV) return; // vite: nothing to persist
    scale.set(before);
    wallpaperChangeFailed.set(true);
  }
}

/// Add a user image and select it. Live: the OS file picker → `add_wallpaper`.
/// Under vite it appends a fixture tile so the flow is verifiable.
export async function addWallpaper(): Promise<void> {
  try {
    const id = await invoke<string>("add_wallpaper");
    await listWallpapers();
    void setWallpaper(id);
  } catch {
    added += 1;
    const id = `wp-added-${added}`;
    const hue = (added * 67) % 360;
    wallpapers.update((list) => [
      ...list,
      { id, name: `Your image ${added}`, thumb: `linear-gradient(135deg, hsl(${hue} 60% 25%), hsl(${(hue + 40) % 360} 60% 45%))`, kind: "static" },
    ]);
    void setWallpaper(id);
  }
}
