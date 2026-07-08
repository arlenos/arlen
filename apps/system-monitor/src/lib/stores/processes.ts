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

/// Load the process list. Live: `list_processes`; fixture under vite.
export async function load(): Promise<void> {
  try {
    processes.set(await invoke<Process[]>("list_processes"));
  } catch {
    processes.set(FIXTURE);
  }
}

/// Gracefully stop a process (SIGTERM ladder), then drop it. Live: `stop_process`.
export async function stop(id: number): Promise<void> {
  processes.update((list) =>
    list.filter((p) => p.id !== id).map((p) =>
      p.children ? { ...p, children: p.children.filter((c) => c.id !== id) } : p,
    ),
  );
  try {
    await invoke("stop_process", { id });
  } catch {
    // optimistic under vite
  }
}
