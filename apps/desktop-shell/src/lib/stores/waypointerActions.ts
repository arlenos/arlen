/// Waypointer Tauri command wrappers.
/// All invoke() calls live here to avoid Tailwind Vite plugin parse errors
/// in .svelte route files.

import { invoke } from "@tauri-apps/api/core";
import { raiseRefusal } from "$lib/shellAction";

export interface AppEntry {
    /// The id the launcher and the permission system key on. Passed back on
    /// launch so the shell can tell whether this app is one it should raise
    /// rather than start again (`app-instance-model.md`).
    app_id: string;
    name: string;
    exec: string;
    icon_name: string;
    icon_data: string | null;
    description: string;
    categories: string[];
}

export interface WaypointerResult {
    result_type: string;
    display: string;
    copy_value: string;
}

export async function fetchAllApps(): Promise<AppEntry[]> {
    return invoke<AppEntry[]>("get_apps");
}

export async function searchApps(query: string): Promise<AppEntry[]> {
    return invoke<AppEntry[]>("search_apps", { query });
}

/// Launch, and say so if it did not.
///
/// The launcher closes on this - correctly, since a person who pressed an app
/// wants the app - so the refusal cannot render here: it goes to the top bar
/// through `raiseRefusal`, the way the quick actions already do. It used to be a
/// fire-and-forget `invoke`, so an app that failed to start left an empty
/// desktop and no sentence anywhere, which is the launcher's most-used action
/// failing in its quietest possible way.
export function launchApp(exec: string, appId?: string, appName?: string) {
    invoke("launch_app", { exec, appId: appId ?? null }).catch(() =>
        raiseRefusal("sh.wp.errLaunch", { app: appName ?? appId ?? exec }),
    );
}

export async function evaluateInput(input: string): Promise<WaypointerResult | null> {
    return invoke<WaypointerResult | null>("evaluate_waypointer_input", { input });
}

/// The three that ACT, and they return their promise on purpose.
///
/// They used to be fire-and-forget `void` calls, and the launcher closed on the
/// same tick it made them - so a command that could not start, a URL that would
/// not open or a search that never left said nothing, on the one surface that
/// had already gone. The module-result branch in `WaypointerContent` had learnt
/// this and awaited its own three; these four callers had not. Returning the
/// promise is what lets a caller keep the window open and say so.
export function executeShellCommand(command: string, inTerminal: boolean): Promise<unknown> {
    return invoke("execute_shell_command", { command, inTerminal });
}

export function openUrl(url: string): Promise<unknown> {
    const full = /^https?:\/\//i.test(url) ? url : `https://${url}`;
    return invoke("open_url", { url: full });
}

export function webSearch(query: string): Promise<unknown> {
    const encoded = encodeURIComponent(query);
    return invoke("open_url", { url: `https://duckduckgo.com/?q=${encoded}` });
}
