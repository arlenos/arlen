import { invoke } from "@tauri-apps/api/core";
import { get, writable } from "svelte/store";
import { formatDecimal, locale } from "@arlen/ui-kit/i18n";

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
    if (bytes < 1024 * 1024 * 1024)
      return `${formatDecimal(bytes / (1024 * 1024), 1, get(locale))} MB`;
    return `${formatDecimal(bytes / (1024 * 1024 * 1024), 1, get(locale))} GB`;
}
