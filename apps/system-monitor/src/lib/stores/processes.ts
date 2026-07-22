/// The task manager's process model (system-monitor-plan.md). The landing is the
/// process list - what is running, sorted so the hog is on top, a Stop on every
/// row. No verdict page. Apps are grouped (one "Firefox" row over its children);
/// the Arlen daemons + the AI agent are ORDINARY rows in the Background group with
/// live CPU/RAM/disk/net - sovereignty made by being an ordinary row, not a lecture.
///
/// Mock-vs-live: fixture-backed. The real process data + Stop/Restart/Limit ride the
/// coder's Rust collection sidecar over the capability-gated read; under vite the
/// store serves a fixture.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// Which group a process lives in.
export type ProcGroup = "app" | "background" | "system";
/// Status in plain words.
export type ProcStatus = "running" | "not-responding" | "suspended";
/// The sortable columns.
export type SortKey = "name" | "status" | "cpu" | "memMB" | "diskKBs" | "netKBs";

/// One process row. An app row aggregates its `children` (per-PID).
export interface Process {
  id: number;
  name: string;
  group: ProcGroup;
  status: ProcStatus;
  cpu: number;
  memMB: number;
  diskKBs: number;
  netKBs: number;
  /// Frozen (cgroup.freeze) - the non-destructive pause.
  paused?: boolean;
  /// Soft-throttled (cgroup memory.high + cpu.max) - the non-destructive leash.
  limited?: boolean;
  children?: Process[];
}

const FIXTURE: Process[] = [
  {
    id: 1,
    name: "Firefox",
    group: "app",
    status: "running",
    cpu: 18.4,
    memMB: 2140,
    diskKBs: 120,
    netKBs: 340,
    children: [
      { id: 101, name: "Arlen OS - Wikipedia", group: "app", status: "running", cpu: 8.0, memMB: 720, diskKBs: 40, netKBs: 180 },
      { id: 102, name: "Design docs", group: "app", status: "running", cpu: 6.4, memMB: 810, diskKBs: 30, netKBs: 90 },
      { id: 103, name: "Mail", group: "app", status: "running", cpu: 4.0, memMB: 610, diskKBs: 50, netKBs: 70 },
    ],
  },
  { id: 2, name: "Meet", group: "app", status: "running", cpu: 9.1, memMB: 920, diskKBs: 8, netKBs: 210 },
  { id: 3, name: "Slack", group: "app", status: "not-responding", cpu: 0.0, memMB: 540, diskKBs: 0, netKBs: 0 },
  { id: 4, name: "Files", group: "app", status: "running", cpu: 1.2, memMB: 180, diskKBs: 22, netKBs: 0 },
  { id: 5, name: "Text editor", group: "app", status: "running", cpu: 0.8, memMB: 240, diskKBs: 4, netKBs: 0 },

  { id: 20, name: "knowledge", group: "background", status: "running", cpu: 3.2, memMB: 410, diskKBs: 64, netKBs: 4 },
  { id: 21, name: "ai-agent", group: "background", status: "running", cpu: 2.1, memMB: 360, diskKBs: 6, netKBs: 12 },
  { id: 22, name: "ai-daemon", group: "background", status: "running", cpu: 1.4, memMB: 300, diskKBs: 2, netKBs: 8 },
  { id: 23, name: "event-bus", group: "background", status: "running", cpu: 0.6, memMB: 90, diskKBs: 1, netKBs: 0 },
  { id: 24, name: "audit-daemon", group: "background", status: "running", cpu: 0.3, memMB: 70, diskKBs: 12, netKBs: 0 },
  { id: 25, name: "modulesd", group: "background", status: "running", cpu: 0.2, memMB: 110, diskKBs: 0, netKBs: 0 },
  { id: 26, name: "notification-daemon", group: "background", status: "running", cpu: 0.1, memMB: 60, diskKBs: 0, netKBs: 0 },

  { id: 40, name: "cosmic-comp", group: "system", status: "running", cpu: 6.2, memMB: 680, diskKBs: 2, netKBs: 0 },
  { id: 41, name: "Xwayland", group: "system", status: "running", cpu: 2.8, memMB: 520, diskKBs: 0, netKBs: 0 },
  { id: 42, name: "pipewire", group: "system", status: "running", cpu: 1.1, memMB: 150, diskKBs: 0, netKBs: 0 },
  { id: 43, name: "systemd", group: "system", status: "running", cpu: 0.4, memMB: 40, diskKBs: 1, netKBs: 0 },
];

export const processes = writable<Process[]>([]);

/// True while the list is the FIXTURE, not this machine's real processes. The
/// rows carry names ("Firefox", "systemd") and live-looking CPU/RAM figures, so
/// unlabelled they read as real - and every row offers a Stop.
export const mocked = writable(false);

/// The last action failure, for the surface to show. Empty when all is well.
/// Set only when a real backend refused - see `stop`/`setFlagChecked`.
export const lastError = writable("");

/// Load the process list. Live: `list_processes`; fixture under vite.
///
/// Merged, not replaced. The backend reports neither `limited` (a cgroup
/// `cpu.max` leash it has no field for) nor `paused`, so a blind `set` would drop
/// both on every poll and show a throttled process as unthrottled. `paused` is
/// re-derived from the backend's own status instead of being carried, so it
/// self-corrects when a process is frozen or thawed outside this app.
export async function load(): Promise<void> {
  try {
    const next = await invoke<Process[]>("list_processes");
    processes.update((prev) => {
      const wasLimited = new Set(prev.filter((p) => p.limited).map((p) => p.id));
      return next.map((p) => ({
        ...p,
        paused: p.status === "suspended",
        limited: wasLimited.has(p.id),
      }));
    });
    mocked.set(false);
  } catch {
    processes.set(FIXTURE);
    mocked.set(true);
  }
}

let pollTimer: ReturnType<typeof setInterval> | null = null;

/// Poll the process list while the Processes tab is visible.
///
/// Without this the list was loaded exactly once at mount, and since the backend
/// computes CPU% and disk rates as a DELTA against the previous sample - its own
/// doc: "The first call (no previous) reports 0 for the rates" - every row showed
/// 0.0% CPU and 0 KB/s forever. A task manager that never updates is the one
/// thing it must not be.
///
/// Only polls with a real backend: under vite each tick would re-set the fixture
/// and wipe the optimistic Stop/Pause the mock relies on to stay reviewable.
export function startProcessPolling(intervalMs = 2000): void {
  if (pollTimer || !tauriAvailable) return;
  pollTimer = setInterval(() => void load(), intervalMs);
}

/// Stop polling (tab hidden or view destroyed).
export function stopProcessPolling(): void {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

/// Gracefully stop a process (SIGTERM ladder), then drop it. Live: `stop_process`.
///
/// Optimistic, but NEVER silently: with a real backend a refused stop is put
/// BACK in the list. Dropping the row and swallowing the error would tell the
/// user they killed a process that is still running - a false confirmation of a
/// destructive action, the one thing this surface must not do.
export async function stop(id: number): Promise<void> {
  let previous: Process[] = [];
  processes.update((list) => {
    previous = list;
    return list.filter((p) => p.id !== id).map((p) =>
      p.children ? { ...p, children: p.children.filter((c) => c.id !== id) } : p,
    );
  });
  try {
    await invoke("stop_process", { id });
  } catch (e) {
    if (tauriAvailable) {
      processes.set(previous);
      lastError.set(`Could not stop that process: ${String(e)}`);
    }
    // Without the runtime there is no backend to refuse: keep the optimistic
    // mock so the surface stays reviewable under vite.
  }
}

function setFlag(id: number, patch: Partial<Process>): void {
  processes.update((list) => list.map((p) => (p.id === id ? { ...p, ...patch } : p)));
}

/// Apply a flag optimistically, then reconcile with the backend: a REAL refusal
/// puts the flag back and says so, rather than leaving the row claiming a state
/// (paused, limited) the kernel never applied. Without the Tauri runtime there is
/// no backend to refuse, so the optimistic mock stands.
async function setFlagChecked(
  id: number,
  patch: Partial<Process>,
  revert: Partial<Process>,
  cmd: string,
  args: Record<string, unknown>,
  failure: string,
): Promise<void> {
  setFlag(id, patch);
  try {
    await invoke(cmd, args);
  } catch (e) {
    if (tauriAvailable) {
      setFlag(id, revert);
      lastError.set(`${failure}: ${String(e)}`);
    }
  }
}

/// Freeze a process (cgroup.freeze) - the non-destructive pause. Live: `freeze_process`.
export async function pause(id: number): Promise<void> {
  await setFlagChecked(
    id, { paused: true }, { paused: false },
    "freeze_process", { id, paused: true }, "Could not pause that process",
  );
}
/// Unfreeze it. Live: `freeze_process`.
export async function resume(id: number): Promise<void> {
  await setFlagChecked(
    id, { paused: false }, { paused: true },
    "freeze_process", { id, paused: false }, "Could not resume that process",
  );
}
/// Soft-throttle a process (cgroup memory.high + cpu.max). Live: `limit_process`.
export async function limit(id: number): Promise<void> {
  await setFlagChecked(
    id, { limited: true }, { limited: false },
    "limit_process", { id, limited: true }, "Could not limit that process",
  );
}
/// Remove the throttle. Live: `limit_process`.
export async function unlimit(id: number): Promise<void> {
  await setFlagChecked(
    id, { limited: false }, { limited: true },
    "limit_process", { id, limited: false }, "Could not remove that limit",
  );
}
