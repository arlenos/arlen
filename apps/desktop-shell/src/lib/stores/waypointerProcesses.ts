import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";

export interface ProcessInfo {
    pid: number;
    name: string;
    memory_bytes: number;
}

/// Filtered process results for Waypointer kill mode.
export const processResults = writable<ProcessInfo[]>([]);

/// Fetches process list and filters by query.
export function updateProcessResults(filter: string) {
    invoke<ProcessInfo[]>("get_processes")
        .then((procs) => {
            if (!filter) {
                processResults.set(procs.slice(0, 15));
                return;
            }
            const lower = filter.toLowerCase();
            processResults.set(
                procs
                    .filter((p) => p.name.toLowerCase().includes(lower))
                    .slice(0, 15)
            );
        })
        .catch(() => { processResults.set([]); });
}

/// Clears process results.
export function clearProcessResults() {
    processResults.set([]);
}

/// Kills a process. force=true sends SIGKILL, false sends SIGTERM.
export async function killProcess(pid: number, force: boolean): Promise<void> {
    await invoke("kill_process", { pid, force });
}

/// Formats bytes to human-readable string.
export function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
