/// Per-process detail (system-monitor-plan.md): the double-click view. The standard
/// tabs (Statistics / Memory / Open files) plus the Arlen-native ACCESS tab - what a
/// process holds (camera/mic, files, sockets) and, native to Arlen, the KG
/// capability scopes it holds with a Revoke right there. The sovereign angle as
/// per-process detail, not a landing.
///
/// Mock-vs-live: statistics, memory and open files are the host's or unmeasured;
/// the Access data (the LCG Grant nodes + the audit ledger) and the revoke are
/// coder seams, and only without a host does the hand-keyed table below stand in.

import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";
import type { Process } from "./processes";

/// One socket the process holds, as the kernel's own tables report it.
export interface Connection {
  proto: string;
  local: string;
  peer: string;
  state: string;
}

/// What a process holds open, read from `/proc/<pid>/fd`. Every field is
/// optional and `undefined` means NOT MEASURED, which is a different statement
/// from an empty list: a process belonging to another user cannot be read at
/// all, and printing "no open files" for it would be a false all-clear on the
/// one screen that exists to say what programs can reach.
/// NB `| null`, not merely optional. serde writes a Rust `None` as JSON `null`,
/// so these arrive as `null` rather than absent, and a `!== undefined` test
/// passes for a field that was never measured. Test with `== null`.
export interface HeldResources {
  openFiles?: string[] | null;
  connections?: Connection[] | null;
  camera?: boolean | null;
  mic?: boolean | null;
  unreadable?: Unread | null;
}

/// Why a read came back with nothing, as a word the catalogue can say. The
/// host answers `unreadable` with a sentence of its own ("not measured: the
/// process has exited"); `readUnread` turns the finite set it sends into one of
/// these, and anything else into `other`, so the pane never prints the host.
export type Unread = "noHost" | "exited" | "otherUser" | "status" | "files" | "other";

const UNREAD_MARKERS: [RegExp, Unread][] = [
  [/has exited/, "exited"],
  [/another user/, "otherUser"],
  [/status could not/, "status"],
  [/file table/, "files"],
];

/// The reason word for a host's `unreadable` sentence.
export function readUnread(text: string | null | undefined): Unread | null {
  if (text == null) return null;
  for (const [marker, word] of UNREAD_MARKERS) if (marker.test(text)) return word;
  console.warn("system-monitor: unrecognised unreadable sentence", text);
  return "other";
}

/// The host names a process state with a word of its own ("Waiting for disk");
/// the pane says it in the reader's language.
const STATE_KEYS: Record<string, string> = {
  Running: "tm.dp.state.running",
  Sleeping: "tm.dp.state.sleeping",
  "Waiting for disk": "tm.dp.state.disk",
  Stopped: "tm.dp.state.stopped",
  "Stopped by a debugger": "tm.dp.state.debugger",
  "Not responding": "tm.dp.state.notResponding",
  Idle: "tm.dp.state.idle",
};

/// The message key for a host's state word.
export function stateKey(state: string): string {
  return STATE_KEYS[state] ?? "tm.dp.state.unknown";
}

/// The Statistics and Memory figures. Same `| null` discipline as above: a
/// number that could not be read arrives as `null`, and the pane must say so
/// rather than print a plausible default. The invented version computed threads
/// as memory divided by 40 and context switches as `1000 + pid * 137`.
export interface ProcStats {
  ppid?: number | null;
  threads?: number | null;
  state?: string | null;
  nice?: number | null;
  ctxSwitches?: number | null;
  rssMB?: number | null;
  pssMB?: number | null;
  sharedMB?: number | null;
  unreadable?: Unread | null;
}

/// Ask the backend for `pid`'s statistics.
export async function statsFor(pid: number): Promise<ProcStats> {
  if (!tauriAvailable) return { unreadable: "noHost" };
  try {
    const s = await invoke<Omit<ProcStats, "unreadable"> & { unreadable?: string | null }>("process_stats", { pid });
    return { ...s, unreadable: readUnread(s.unreadable) };
  } catch (e) {
    console.warn("system-monitor: process_stats did not answer", e);
    return { unreadable: "other" };
  }
}

/// Ask the backend what `pid` is holding.
///
/// Outside a Tauri webview there is no backend to ask, and the honest answer is
/// that nothing was measured - NOT a fixture. The invented version of this
/// (three paths built from the process name, plus `tcp 140.82.121.4:443
/// ESTABLISHED` for anything with traffic) put a real GitHub address on screen
/// for a process nobody had inspected.
export async function heldFor(pid: number): Promise<HeldResources> {
  if (!tauriAvailable) return { unreadable: "noHost" };
  try {
    const h = await invoke<Omit<HeldResources, "unreadable"> & { unreadable?: string | null }>("process_held_resources", { pid });
    return { ...h, unreadable: readUnread(h.unreadable) };
  } catch (e) {
    console.warn("system-monitor: process_held_resources did not answer", e);
    return { unreadable: "other" };
  }
}

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
/// The per-process detail this store still derives itself: the pid, and the
/// hand-keyed access stand-in below. Statistics, memory and open files come
/// from the host (`statsFor`, `heldFor`) or are said to be unmeasured; the
/// invented versions of those (threads as memory divided by forty, context
/// switches as `1000 + pid * 137`, a GitHub address for anything with traffic)
/// are gone rather than carried unread.
export interface ProcDetail {
  pid: number;
  access: ProcAccess;
}

// Access is keyed by known process name so it reads meaningfully; everything else
// gets the honest minimal default.
//
// These sentences are ours and they are not translated, deliberately: the table
// is a stand-in. What a process may reach is already derived properly from its
// permission profile in Settings (`stores/grants.ts` turns a `GrantView` into
// scope lines), and this hand-keyed copy will be replaced by that rather than
// grown. Translating it now is work that dies with the table; what it needs is
// the profile behind it.
const ACCESS: Record<string, ProcAccess> = {
  Meet: {
    camera: true,
    mic: true,
    reach: "It can use the network, the microphone and the camera.",
    scopes: [],
  },
  "ai-agent": {
    camera: false,
    mic: false,
    reach: "It reads from the knowledge graph within its granted scope and writes nothing without your say.",
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

/// Nothing measured: what the table and the pane say about a process on a host
/// until a real reader exists. Never the hand-keyed table, whose rows are
/// matched on a NAME, so a real process called `knowledge` would inherit the
/// sample's scopes as its own.
const UNMEASURED_ACCESS: ProcAccess = { camera: false, mic: false, reach: "", scopes: [] };

/// The sensitive access a process holds, for the process-list Access column:
/// the physical sensors (camera/mic) + whether it holds knowledge-graph access.
/// With a host: nothing, because nothing measures it yet and the column says so.
export function sensorsFor(name: string): { camera: boolean; mic: boolean; knowledge: boolean } {
  const a = tauriAvailable ? undefined : ACCESS[name];
  return {
    camera: a?.camera ?? false,
    mic: a?.mic ?? false,
    knowledge: (a?.scopes.length ?? 0) > 0,
  };
}

/// The detail for a process. Without a host the access is the labelled
/// stand-in; with one it is unmeasured until the profile reader exists.
export function detailFor(p: Process): ProcDetail {
  return {
    pid: p.id,
    access: tauriAvailable ? UNMEASURED_ACCESS : (ACCESS[p.name] ?? DEFAULT_ACCESS),
  };
}
