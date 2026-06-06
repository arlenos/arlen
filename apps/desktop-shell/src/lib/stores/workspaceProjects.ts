// Value + type imports split — inline mixed form (`import { x, type Y }`)
// trips the Tailwind Vite plugin's CSS parser. See top-level CLAUDE.md.
import { derived, writable } from "svelte/store";
import type { Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { windows } from "./windows.js";
import { primaryWorkspaces } from "./workspaces.js";

/// Minimal project shape used by the WorkspaceIndicator overlay.
/// Matches the Rust `ProjectInfo` struct (camelCase serde rename).
export interface ProjectInfo {
    id: string;
    name: string;
    rootPath: string;
}

// ── Internal cache ──────────────────────────────────────────────────────
//
// CRITICAL: the cache is a PLAIN JS Map, not a Svelte writable. Earlier
// iterations used `projectByApp = writable(new Map())` and then called
// `projectByApp.update((m) => { ...; return m; })` to read inside the
// derived. That triggered an infinite reactive loop:
//
//   windows updates → projectPerWorkspace re-runs
//     → ensureProjectForApp(appId) for each window
//       → projectByApp.update(...)   ← notifies subscribers EVEN if the
//                                       returned reference is unchanged
//         → projectPerWorkspace re-runs
//           → ensureProjectForApp(...) again ...
//
// Svelte detects the cycle as `effect_update_depth_exceeded` and freezes
// the UI rather than crashing — which manifested as the whole topbar
// going dead after opening a new app (see debug session 2026-04-19).
//
// Now: the Map is plain, mutations don't notify anyone synchronously,
// and we tick a separate `cacheVersion` writable inside the async
// resolution callback. The derived re-runs at most once per resolved
// app_id, never recursively.
const projectByApp = new Map<string, ProjectInfo | null>();
const pending = new Set<string>();
const cacheVersion = writable(0);

/// Fire-and-forget: if `appId` hasn't been queried yet, kick off the
/// backend lookup and bump `cacheVersion` once the answer lands. Safe
/// to call from inside a derived store — no synchronous store writes.
function ensureProjectForApp(appId: string): void {
    if (!appId) return;
    if (projectByApp.has(appId) || pending.has(appId)) return;
    pending.add(appId);
    invoke<ProjectInfo | null>("get_project_for_app", { appId })
        .then((info) => {
            projectByApp.set(appId, info);
            // Tick *after* the cache is populated so any derived that
            // re-runs in response sees the fresh entry.
            cacheVersion.update((v) => v + 1);
        })
        .catch((e) => console.warn("get_project_for_app failed", appId, e))
        .finally(() => pending.delete(appId));
}

/// Derived: map of `workspace_id → ProjectInfo | null`. A workspace
/// gets a project label when ONE project holds strictly more than
/// 50% of the workspace's windows (spec §4.3). Ambiguous workspaces
/// (tie, no majority, all apps without projects) get `null` so the
/// UI renders no label.
///
/// Reactivity model:
///   • `windows` / `primaryWorkspaces` change → recompute votes.
///   • `cacheVersion` ticks (after a `get_project_for_app` resolves)
///     → recompute with the newly cached app→project mapping.
///   • Inside the body: read from the plain `projectByApp` Map (NOT
///     a store — no subscription, no reactive churn).
///   • `ensureProjectForApp` calls inside the body never write to
///     any store synchronously, so they cannot retrigger this same
///     derived in a loop.
export const projectPerWorkspace: Readable<Map<string, ProjectInfo | null>> =
    derived(
        [windows, primaryWorkspaces, cacheVersion],
        ([$windows, $primaryWorkspaces, _$tick]) => {
            const result = new Map<string, ProjectInfo | null>();

            for (const ws of $primaryWorkspaces) {
                const wsWindows = $windows.filter((w) =>
                    w.workspace_ids.includes(ws.id),
                );
                if (wsWindows.length === 0) {
                    result.set(ws.id, null);
                    continue;
                }

                // Kick off any missing cache entries. Cache resolution
                // is async; the next `cacheVersion` tick will bring us
                // back here with fresh data.
                for (const w of wsWindows) ensureProjectForApp(w.app_id);

                // Tally project votes across windows on this workspace.
                const counts = new Map<string, number>();
                const meta = new Map<string, ProjectInfo>();
                for (const w of wsWindows) {
                    const p = projectByApp.get(w.app_id);
                    if (!p) continue; // null = no project, undefined = pending
                    counts.set(p.id, (counts.get(p.id) ?? 0) + 1);
                    meta.set(p.id, p);
                }

                let bestId: string | null = null;
                let bestCount = 0;
                for (const [id, c] of counts) {
                    if (c > bestCount) {
                        bestId = id;
                        bestCount = c;
                    }
                }

                // Strict majority: >50% of all windows on this
                // workspace must vote for the same project. Matches
                // spec §4.3 ("Wenn ein Projekt >50% der Fenster hat").
                const majority = bestCount > wsWindows.length / 2;
                result.set(
                    ws.id,
                    majority && bestId ? meta.get(bestId)! : null,
                );
            }

            return result;
        },
    );
