/// Per-process detail (system-monitor-plan.md): the double-click view. The standard
/// tabs (Statistics / Memory / Open files) plus the Arlen-native ACCESS tab - what a
/// process holds (camera/mic, files, sockets) and, native to Arlen, the KG
/// capability scopes it holds with a Revoke right there. The sovereign angle as
/// per-process detail, not a landing.
///
/// Mock-vs-live: fixture-backed. The real detail (/proc, ss, open files) + the Access
/// data (rides sdk/system-monitor + the LCG Grant nodes + the audit ledger) + the
/// revoke are coder seams; under vite this derives a plausible fixture.

import { invoke } from "@tauri-apps/api/core";
import type { Process } from "./processes";

/// One held KG capability scope, revocable right here.
export interface AccessScope {
  label: string;
  reach: string;
}
/// What a process can reach - the sovereign summary.
export interface ProcAccess {
  camera: boolean;
  mic: boolean;
  reach: string;
  scopes: AccessScope[];
}
/// The full per-process detail.
export interface ProcDetail {
  pid: number;
  ppid: number;
  threads: number;
  state: string;
  priority: number;
  ctxSwitches: number;
  rssMB: number;
  pssMB: number;
  sharedMB: number;
  openFiles: string[];
  connections: string[];
  access: ProcAccess;
}

// Access is keyed by known process name so it reads meaningfully; everything else
// gets the honest minimal default.
const ACCESS: Record<string, ProcAccess> = {
  Meet: {
    camera: true,
    mic: true,
    reach: "It can use the network, the microphone, and the camera.",
    scopes: [],
  },
  "ai-agent": {
    camera: false,
    mic: false,
    reach: "It reads from the knowledge graph within its granted scope, and writes nothing without your say.",
    scopes: [
      { label: "read your notes", reach: "notes and their tags" },
      { label: "read recent files", reach: "files you opened this week" },
    ],
  },
  "ai-daemon": {
    camera: false,
    mic: false,
    reach: "It answers your questions from the graph.",
    scopes: [{ label: "read the knowledge graph", reach: "the query tier you set" }],
  },
  knowledge: {
    camera: false,
    mic: false,
    reach: "It maintains the knowledge graph.",
    scopes: [{ label: "read and write the graph", reach: "the whole graph" }],
  },
  Firefox: {
    camera: false,
    mic: false,
    reach: "It can use the network and your Downloads folder.",
    scopes: [],
  },
  Files: {
    camera: false,
    mic: false,
    reach: "It can reach the folders you open in it.",
    scopes: [],
  },
};
const DEFAULT_ACCESS: ProcAccess = {
  camera: false,
  mic: false,
  reach: "It runs with limited access and holds nothing sensitive.",
  scopes: [],
};

/// The camera/mic sensors a process holds, for the process-list Access column.
export function sensorsFor(name: string): { camera: boolean; mic: boolean } {
  const a = ACCESS[name];
  return { camera: a?.camera ?? false, mic: a?.mic ?? false };
}

/// Derive the detail for a process (a fixture; the real data is the sidecar seam).
export function detailFor(p: Process): ProcDetail {
  const lower = p.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  return {
    pid: p.id,
    ppid: p.group === "system" ? 1 : 1200 + (p.id % 40),
    threads: Math.max(1, Math.round(p.memMB / 40)),
    state:
      p.status === "not-responding" ? "Uninterruptible sleep" : p.status === "suspended" ? "Stopped" : "Running",
    priority: p.group === "system" ? 0 : 20,
    ctxSwitches: 1000 + p.id * 137,
    rssMB: p.memMB,
    pssMB: Math.round(p.memMB * 0.82),
    sharedMB: Math.round(p.memMB * 0.18),
    openFiles: [
      `/home/tim/.config/arlen/${lower}.toml`,
      `/proc/${p.id}/status`,
      `/run/user/1000/arlen/${lower}.sock`,
    ],
    connections: p.netKBs > 0 ? ["tcp 140.82.121.4:443 ESTABLISHED", "udp 224.0.0.251:5353 mdns"] : [],
    access: ACCESS[p.name] ?? DEFAULT_ACCESS,
  };
}

/// Revoke a held scope from a process (the monitor notices; the App-access page
/// holds the standing revoke). Live: `revoke_scope`.
export async function revokeScope(id: number, label: string): Promise<void> {
  try {
    await invoke("revoke_scope", { id, label });
  } catch {
    // optimistic under vite
  }
}
