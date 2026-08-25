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
import { refreshMs } from "$lib/refresh";

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
  /// Something else depends on this one, so Stop asks first. Decided in the core
  /// (`procmon::is_critical`) and carried on the row, never re-derived from the
  /// name here: two lists of daemon names in two languages is how a guardrail
  /// quietly stops covering the daemon somebody renamed.
  critical?: boolean;
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

/// True when a real session could not read the process list at all.
export const unavailable = writable(false);

/// Why the last action on a process did not happen: a message id and its values,
/// NOT a sentence. Null when all is well.
///
/// Seven places in this file used to compose the sentence here - "Could not stop
/// that process: " plus whatever Rust said - which put the words a person reads
/// into a TypeScript store where no catalogue can reach them, and left a German
/// build explaining somebody's own machine to them in English. The page writes
/// the sentence now, from these.
///
/// Set only when a real backend refused - see `stop`/`setFlagChecked`.
export const lastError = writable<
  { key: string; values: Record<string, unknown>; reason: string } | null
>(null);

/// The refusal tokens `stop_process` / `freeze_process` / `limit_process` /
/// `renice_process` answer with, to the key that says each one in the reader's
/// language.
///
/// The other half of the sentence, and it was the half still in English. The
/// action clause moved into the catalogue and `reason` kept carrying whatever
/// Rust said, which for the common case was `strerror`: press End process on a
/// system service in a German build and the page read "Dieser Prozess ließ sich
/// nicht beenden: Operation not permitted (os error 1)". That is the most likely
/// refusal on this surface, not an edge, and priority is worse - lowering a nice
/// value ALWAYS needs `CAP_SYS_NICE`, so an ordinary user asking for Highest
/// meets EPERM every time.
const WHY: Record<string, string> = {
  "unsafe-pid": "sm.why.unsafePid",
  "not-permitted": "sm.why.notPermitted",
  gone: "sm.why.gone",
  "bad-priority": "sm.why.badPriority",
  "no-cgroup": "sm.why.noCgroup",
  "no-delegation": "sm.why.noDelegation",
  other: "sm.why.other",
};

/// The message key for a refusal.
///
/// An unrecognised value logs and falls back to the vague sentence rather than
/// being shown: a token this does not know is a backend that changed, and the
/// developer wants to read that in the console, not the user on the page.
function whyKey(e: unknown): string {
  const token = String(e);
  const key = WHY[token];
  if (key) return key;
  console.warn("system-monitor: unrecognised refusal token", token);
  return "sm.why.other";
}

/// Load the process list. Live: `list_app_rows`; fixture under vite.
///
/// The GROUPED rows, because that is what the plan says the landing opens on -
/// one "chrome" row over its children rather than fifteen nameless pids. The
/// flat `list_processes` is still there and is what the power-user toggle will
/// ask for; both return the same `Process` shape, the grouped one simply
/// carrying `children`.
///
/// Merged, not replaced. The backend reports neither `limited` (a cgroup
/// `cpu.max` leash it has no field for) nor `paused`, so a blind `set` would drop
/// both on every poll and show a throttled process as unthrottled. `paused` is
/// re-derived from the backend's own status instead of being carried, so it
/// self-corrects when a process is frozen or thawed outside this app.
export async function load(): Promise<void> {
  try {
    const next = await invoke<Process[]>("list_app_rows");
    processes.update((prev) => {
      const wasLimited = new Set(prev.filter((p) => p.limited).map((p) => p.id));
      return next.map((p) => ({
        ...p,
        paused: p.status === "suspended",
        limited: wasLimited.has(p.id),
      }));
    });
    mocked.set(false);
    unavailable.set(false);
    // CPU, disk and network are DELTAS against the previous sample, so the first
    // poll of a run has nothing to subtract and the backend reports them as
    // zero. Rendering that zero says "this process is using no CPU", which is a
    // measurement nobody took - and at a 10s refresh rate it is on screen for
    // ten seconds. The Performance tab already had this distinction as
    // `ratesReady`; the process list did not, and every row read 0.0% until the
    // second poll landed.
    loads += 1;
    if (loads >= 2) ratesReady.set(true);
  } catch {
    if (!tauriAvailable) {
      processes.set(FIXTURE);
      mocked.set(true);
      unavailable.set(false);
      return;
    }
    // The fixture does not just describe processes, it supplies their ids - 1,
    // 101, 102, 103 - and those ids are the argument to `stop_process`. Every
    // other invented list tonight was wrong on screen; this one hands a real
    // PID to a destructive call, and 1 is init. `stop()` above reasons
    // carefully about never giving a false confirmation of a destructive
    // action, which is the same concern arriving one step too late.
    processes.set([]);
    mocked.set(false);
    unavailable.set(true);
  }
}

/// How many successful polls this run has had. Two are needed before any rate
/// column means anything.
let loads = 0;

/// False until a delta exists. The table renders the rate columns as unmeasured
/// while it is false rather than printing the backend's placeholder zero.
///
/// Starts TRUE without a Tauri runtime: under vite the fixture is the data, it
/// is not a delta of anything, and blanking its CPU column would make the mock
/// unreviewable while telling the truth about nothing.
export const ratesReady = writable(!tauriAvailable);

let pollTimer: ReturnType<typeof setInterval> | null = null;
let rateUnsub: (() => void) | null = null;

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
export function startProcessPolling(intervalMs?: number): void {
  if (pollTimer || !tauriAvailable) return;
  if (intervalMs !== undefined) {
    pollTimer = setInterval(() => void load(), intervalMs);
    return;
  }
  // Follows the shared rate (system-monitor-plan.md (a)). Subscribing rather
  // than reading once means changing the rate takes effect on the running view,
  // not at the next mount - a control that only applies after a restart is one
  // people conclude is broken.
  rateUnsub = refreshMs.subscribe((ms) => {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = setInterval(() => void load(), ms);
  });
}

/// Stop polling (tab hidden or view destroyed).
export function stopProcessPolling(): void {
  // The backend keeps its own previous sample across a stop/start, so readiness
  // is NOT reset here: re-showing the tab would otherwise blank every rate
  // column for one poll even though the delta is available.
  if (rateUnsub) {
    rateUnsub();
    rateUnsub = null;
  }
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
      lastError.set({ key: "sm.err.stop", values: {}, reason: whyKey(e) });
    }
    // Without the runtime there is no backend to refuse: keep the optimistic
    // mock so the surface stays reviewable under vite.
  }
}

/// Every pid a row stands for: itself and, when it is an app row, its members.
///
/// The landing view became app-grouped on 18 August and this was the hole it
/// opened. A "chrome" row is fifteen processes with the eldest pid on the row, so
/// `stop(row.id)` ended ONE of them and left fourteen - the app still on screen,
/// the row back on the next poll, and the person told it was stopped. The plan is
/// explicit: "On an app group, Stop terminates the whole process tree."
export function pidsOf(p: Process): number[] {
  return [p.id, ...(p.children ?? []).map((c) => c.id)];
}

/// Stop everything a row stands for.
///
/// Each pid is stopped on its own rather than as one call, because a partial
/// refusal is the normal case: a browser's helpers may go while a privileged
/// child stays. Any refusal RELOADS rather than restoring the pre-click list -
/// with a group, some are gone and some are not, and putting the old list back
/// would claim the survivors are all still there.
export async function stopRow(p: Process): Promise<void> {
  const ids = pidsOf(p);
  if (ids.length === 1) return stop(ids[0]);
  const failures: unknown[] = [];
  for (const id of ids) {
    try {
      await invoke("stop_process", { id });
    } catch (e) {
      failures.push(e);
    }
  }
  if (failures.length && tauriAvailable) {
    lastError.set({
      key: "sm.err.stopSome",
      values: { done: ids.length - failures.length, total: ids.length },
      // The FIRST refusal, because a group usually meets one reason: the
      // privileged members refuse and the rest go. Listing seven identical
      // clauses would say less than one.
      reason: whyKey(failures[0]),
    });
  }
  // Reload either way: the truth about which members survived is the backend's,
  // and after a group action guessing it locally is exactly the false
  // confirmation this surface must not give.
  await load();
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
      lastError.set({ key: failure, values: {}, reason: whyKey(e) });
    }
  }
}

/// Apply a lever to every process a row stands for.
///
/// The same hole `stopRow` closes, and worse in effect. Stop at least ended
/// something a person could see go; `pause(row.id)` froze ONE of fifteen chrome
/// processes and set the row to "suspended", so the app carried on running under
/// a label saying it was frozen. The plan calls Pause "freezes the app group
/// atomically" and it is the lever meant to be reached for FIRST, since it is
/// the reversible one.
///
/// The flag goes on the row optimistically and comes back off if any member
/// refused - a group that is half-frozen is not paused, and saying so is the
/// point. The message names how many took it.
async function rowLever(
  row: Process,
  patch: Partial<Process>,
  revert: Partial<Process>,
  cmd: string,
  argsFor: (id: number) => Record<string, unknown>,
  failure: string,
): Promise<void> {
  const ids = pidsOf(row);
  setFlag(row.id, patch);
  const failures: unknown[] = [];
  for (const id of ids) {
    try {
      await invoke(cmd, argsFor(id));
    } catch (e) {
      failures.push(e);
    }
  }
  if (failures.length && tauriAvailable) {
    setFlag(row.id, revert);
    lastError.set({
      key: `${failure}.some`,
      values: { done: ids.length - failures.length, total: ids.length },
      reason: whyKey(failures[0]),
    });
  }
}

/// Freeze every process the row stands for. Live: `freeze_process`.
export async function pauseRow(row: Process): Promise<void> {
  await rowLever(row, { paused: true }, { paused: false }, "freeze_process",
    (id) => ({ id, paused: true }), "sm.err.pauseApp");
}

/// Thaw every process the row stands for.
export async function resumeRow(row: Process): Promise<void> {
  await rowLever(row, { paused: false }, { paused: true }, "freeze_process",
    (id) => ({ id, paused: false }), "sm.err.resumeApp");
}

/// Throttle every process the row stands for.
export async function limitRow(row: Process): Promise<void> {
  await rowLever(row, { limited: true }, { limited: false }, "limit_process",
    (id) => ({ id, limited: true }), "sm.err.limitApp");
}

/// Remove the throttle from every process the row stands for.
export async function unlimitRow(row: Process): Promise<void> {
  await rowLever(row, { limited: false }, { limited: true }, "limit_process",
    (id) => ({ id, limited: false }), "sm.err.unlimitApp");
}

/// Freeze a process (cgroup.freeze) - the non-destructive pause. Live: `freeze_process`.
export async function pause(id: number): Promise<void> {
  await setFlagChecked(
    id, { paused: true }, { paused: false },
    "freeze_process", { id, paused: true }, "sm.err.pause",
  );
}
/// Unfreeze it. Live: `freeze_process`.
export async function resume(id: number): Promise<void> {
  await setFlagChecked(
    id, { paused: false }, { paused: true },
    "freeze_process", { id, paused: false }, "sm.err.resume",
  );
}
/// Soft-throttle a process (cgroup memory.high + cpu.max). Live: `limit_process`.
export async function limit(id: number): Promise<void> {
  await setFlagChecked(
    id, { limited: true }, { limited: false },
    "limit_process", { id, limited: true }, "sm.err.limit",
  );
}
/// Remove the throttle. Live: `limit_process`.
export async function unlimit(id: number): Promise<void> {
  await setFlagChecked(
    id, { limited: false }, { limited: true },
    "limit_process", { id, limited: false }, "sm.err.unlimit",
  );
}

/// One offered priority: the plain-words label and its nice value.
export interface NiceLevel {
  label: string;
  nice: number;
}

/// The priority levels, from the backend so the list and the validator agree.
export async function niceLevels(): Promise<NiceLevel[]> {
  if (!tauriAvailable) return [];
  try {
    const raw = await invoke<[string, number][]>("nice_levels");
    return raw.map(([label, nice]) => ({ label, nice }));
  } catch {
    return [];
  }
}

/// What priority a process is at now. `null` when it cannot be read - never 0,
/// which would tick "Normal" for a process that is nothing of the sort.
export async function niceOf(id: number): Promise<number | null> {
  if (!tauriAvailable) return null;
  try {
    return (await invoke<number | null>("process_nice", { id })) ?? null;
  } catch {
    return null;
  }
}

/// Change a process's priority (system-monitor-plan.md (c), the Advanced
/// affordance).
///
/// Surfaces a refusal instead of swallowing it, and this one is refused often:
/// raising a nice value is free but lowering one needs `CAP_SYS_NICE`, so an
/// ordinary user asking for a HIGHER priority gets EPERM. Reported rather than
/// ignored, or the menu would look like it worked and change nothing.
export async function renice(id: number, nice: number): Promise<boolean> {
  if (!tauriAvailable) return false;
  try {
    await invoke("renice_process", { id, nice });
    return true;
  } catch (e) {
    lastError.set({ key: "sm.err.priority", values: {}, reason: whyKey(e) });
    return false;
  }
}
