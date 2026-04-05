/// Waypointer Tauri command wrappers.
/// All invoke() calls live here to avoid Tailwind Vite plugin parse errors
/// in .svelte route files.

import { invoke } from "@tauri-apps/api/core";

export interface AppEntry {
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

export function launchApp(exec: string) {
    invoke("launch_app", { exec });
}

export async function evaluateInput(input: string): Promise<WaypointerResult | null> {
    return invoke<WaypointerResult | null>("evaluate_waypointer_input", { input });
}

export function executeShellCommand(command: string, inTerminal: boolean) {
    invoke("execute_shell_command", { command, inTerminal });
}

export function openUrl(url: string) {
    const full = /^https?:\/\//i.test(url) ? url : `https://${url}`;
    invoke("open_url", { url: full });
}

export function webSearch(query: string) {
    const encoded = encodeURIComponent(query);
    invoke("open_url", { url: `https://duckduckgo.com/?q=${encoded}` });
}
