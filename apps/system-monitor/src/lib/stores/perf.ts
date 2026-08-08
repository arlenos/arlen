/// The Performance tab's device series: real CPU, memory, disk and network,
/// sampled from the host once a second.
///
/// This used to be a random walk. It drew a plausible history, moved when you
/// watched it, and none of it was this machine - a task manager showing "Memory
/// 85%" that nobody had measured. The numbers now come from `system_tick`, which
/// reads `/proc/stat`, `/proc/meminfo`, `/proc/diskstats` and `/proc/net/dev` in
/// the host and returns rates, so the collection stays out of the webview (the
/// monitor must never be the top process in its own list).
///
/// Three of the four are rates, so the first tick after opening the tab has
/// nothing to subtract from and reports them as zero with `ratesReady` false. The
/// surface says it is waiting rather than drawing that zero as a measurement.
///
/// There is no fixture on the failure path. If the host cannot be reached the tab
/// says so; inventing a series here is the exact thing this replaced.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// A monitored device.
export type Device = "cpu" | "memory" | "disk" | "network" | "ai";

/// The devices, in list order, with their axis scale. `label` is a message KEY,
/// resolved with `$t` where it renders: a module-level constant captures whatever
/// the translator held at import, so the names would never follow a locale switch.
/// The first four reuse the process table's column keys rather than duplicating
/// the same four words in a second place.
///
/// `max` is the axis ceiling. CPU and memory are percentages; disk and network are
/// MiB/s, where no ceiling is correct for every machine, so the graph is drawn
/// against a scale that grows with what has been seen (see `axisMax`).
export const DEVICES: { key: Device; label: string; max: number }[] = [
  { key: "cpu", label: "tm.col.cpu", max: 100 },
  { key: "memory", label: "tm.col.memory", max: 100 },
  { key: "disk", label: "tm.col.disk", max: 50 },
  { key: "network", label: "tm.col.network", max: 10 },
  { key: "ai", label: "tm.dev.ai", max: 120 },
];

/// One tick as the host reports it. Numbers only: the wording lives in the
/// catalogue, so a backend string would be untranslatable by construction.
export type SystemTick = {
  cpuPct: number;
  cpuCount: number;
  memPct: number;
  memUsedGb: number;
  memTotalGb: number;
  diskReadMbs: number;
  diskWriteMbs: number;
  netRxMbs: number;
  netTxMbs: number;
  /// False on the first tick, when the rates have nothing to delta against.
  ratesReady: boolean;
};

/// How many points the graphs hold.
const CAP = 60;

type Series = Record<Device, number[]>;

const empty = (): Series => ({ cpu: [], memory: [], disk: [], network: [], ai: [] });

/// The rolling series per device. Empty until the first tick lands, and the AI
/// series stays empty: tokens per second is the engine's figure, not a kernel
/// counter, and nothing reports it yet.
export const series = writable<Series>(empty());

/// The most recent tick, or null before the first one.
export const tick = writable<SystemTick | null>(null);

/// Why there are no measurements, when there are none. Null while it is working.
export const perfError = writable<string | null>(null);

/// The axis ceiling for a device: its floor, or the largest value seen, so a
/// machine that briefly does 400 MB/s is not drawn flat against a 50 MB/s ceiling
/// and an idle one is not a flat line at the bottom of a huge scale.
export function axisMax(values: number[], floor: number): number {
  const peak = values.length ? Math.max(...values) : 0;
  return peak > floor ? peak * 1.1 : floor;
}

function push(s: Series, d: Device, v: number): number[] {
  const arr = s[d].length >= CAP ? s[d].slice(1) : s[d].slice();
  arr.push(v);
  return arr;
}

let timer: ReturnType<typeof setInterval> | null = null;

async function sample(): Promise<void> {
  try {
    const t = await invoke<SystemTick>("system_tick");
    tick.set(t);
    perfError.set(null);
    series.update((s) => ({
      ...s,
      // Same rule as disk and network: a rate the host could not compute is not a
      // point on a graph.
      cpu: t.ratesReady ? push(s, "cpu", t.cpuPct) : s.cpu,
      memory: push(s, "memory", t.memPct),
      // A rate the host could not compute yet is not a zero worth drawing.
      disk: t.ratesReady ? push(s, "disk", t.diskReadMbs + t.diskWriteMbs) : s.disk,
      network: t.ratesReady ? push(s, "network", t.netRxMbs + t.netTxMbs) : s.network,
    }));
  } catch (e) {
    perfError.set(String(e));
  }
}

/// Start the 1 Hz sampler. Without a Tauri runtime (a browser, the screenshot
/// loop) there is no host to ask, and the tab says so rather than drawing.
export function startPerf(): void {
  if (timer) return;
  if (!tauriAvailable) {
    perfError.set("no host");
    return;
  }
  void sample();
  timer = setInterval(() => void sample(), 1000);
}

/// Stop the tick when the Performance tab isn't visible.
export function stopPerf(): void {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
}
