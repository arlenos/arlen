/// Waypointer "Recents" store — empty-query landing state.
///
/// Two data sources, loaded in parallel on every Waypointer open:
///
///   • **Recent Apps** — local JSON (`~/.local/share/lunaris/app-history.json`)
///     fed by `record_app_launch` every time the user launches an app
///     from the Waypointer. Returns `AppEntry[]` by joining the stored
///     `exec` keys against the live app index.
///   • **Recent Files** — the Knowledge Graph (`File` nodes ordered by
///     `last_accessed DESC`). Empty list if the daemon is down — we
///     hide the section silently instead of showing an error.
///
/// Both reads are cached upstream (app history is file-local and
/// fast; recent files has a 5s TTL in Rust) so opening the Waypointer
/// repeatedly doesn't hammer disk or the graph socket.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { AppEntry } from "./waypointerActions.js";

export interface RecentFile {
    path: string;
    lastAccessed: number;
}

/// Display-ready derived state. Consumed by `WaypointerContent.svelte`
/// when the query is empty. The two arrays are independent: either can
/// be empty while the other has data.
const recentApps = writable<AppEntry[]>([]);
const recentFiles = writable<RecentFile[]>([]);

export const recentAppsStore: Readable<AppEntry[]> = { subscribe: recentApps.subscribe };
export const recentFilesStore: Readable<RecentFile[]> = { subscribe: recentFiles.subscribe };

/// Load both recent-apps and recent-files in parallel. Callers pass
/// the current full app index (`allApps`) so we can resolve stored
/// `exec` keys into displayable `AppEntry` rows without a second
/// fetch. Safe to call on every Waypointer open; underlying backends
/// cache / dedupe.
export async function loadRecents(
    allApps: AppEntry[],
    opts: { appsLimit?: number; filesLimit?: number } = {},
): Promise<void> {
    const appsLimit = opts.appsLimit ?? 8;
    const filesLimit = opts.filesLimit ?? 6;

    const [execs, files] = await Promise.all([
        invoke<string[]>("get_recent_apps", { limit: appsLimit }).catch(() => []),
        invoke<RecentFile[]>("get_recent_files", { limit: filesLimit }).catch(
            () => [] as RecentFile[],
        ),
    ]);

    // Join exec keys against the in-memory app index. O(n*m) but both
    // n and m are small (≤ 8 recents, a few hundred apps); for typical
    // installs this is <1ms.
    const apps: AppEntry[] = [];
    for (const exec of execs) {
        const match = allApps.find((a) => a.exec === exec);
        if (match) apps.push(match);
    }

    recentApps.set(apps);
    recentFiles.set(files);
}

/// Record an app launch into the history so the next Waypointer open
/// reflects it. Fire-and-forget: failures to persist don't block the
/// actual launch (the launch has already happened upstream).
export function recordAppLaunch(exec: string): void {
    invoke("record_app_launch", { exec }).catch((e) =>
        console.warn("[waypointer] record_app_launch failed:", e),
    );
}

/// Open a recent file via the system's default handler. Fire-and-
/// forget; the Waypointer closes before we'd see any error.
export function openRecentFile(path: string): void {
    invoke("open_recent_file", { path }).catch((e) =>
        console.warn("[waypointer] open_recent_file failed:", e),
    );
}

/// Clear both stores. Called when the Waypointer hides so the next
/// open re-renders from a known-empty state (avoids a flash of stale
/// recents between loads).
export function clearRecents(): void {
    recentApps.set([]);
    recentFiles.set([]);
}
