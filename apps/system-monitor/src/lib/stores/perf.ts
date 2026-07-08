/// The Performance tab's live device data (system-monitor-plan.md). Rolling series
/// for CPU / Memory / Disk / Network + an AI-compute device (the AI's tokens/sec +
/// context - the Arlen-native one). Drawn on canvas, never reactive SVG.
///
/// Mock-vs-live: fixture-backed. The store pushes a plausible synthetic tick ~1 Hz;
/// the REAL pre-aggregated tick stream comes from the coder's Rust collection
/// sidecar (routed through here, never computed in the webview - the "don't make the
/// webview the top process" rule).

import { writable } from "svelte/store";

/// A monitored device.
export type Device = "cpu" | "memory" | "disk" | "network" | "ai";

/// The devices, in list order, with their axis scale.
export const DEVICES: { key: Device; label: string; max: number }[] = [
  { key: "cpu", label: "CPU", max: 100 },
  { key: "memory", label: "Memory", max: 100 },
  { key: "disk", label: "Disk", max: 200 },
  { key: "network", label: "Network", max: 100 },
  { key: "ai", label: "AI compute", max: 120 },
];

const CAP = 60;
type Series = Record<Device, number[]>;
/// The current value + a plain-label detail line per device.
type Stats = Record<Device, { value: string; detail: string }>;

const cur: Record<Device, number> = { cpu: 22, memory: 42, disk: 34, network: 18, ai: 14 };

function step(d: Device, max: number): number {
  let v = cur[d] + (Math.random() - 0.5) * max * 0.16;
  if (Math.random() < 0.05) v += max * 0.3;
  v = Math.max(max * 0.02, Math.min(max * 0.95, v));
  cur[d] = v;
  return v;
}

function seed(): Series {
  const s = {} as Series;
  for (const d of DEVICES) s[d.key] = Array.from({ length: CAP }, () => step(d.key, d.max));
  return s;
}

function fmtStats(): Stats {
  const gb = (pct: number) => `${((pct / 100) * 16).toFixed(1)} / 16 GB`;
  return {
    cpu: { value: `${cur.cpu.toFixed(0)}%`, detail: "8 cores, 16 threads" },
    memory: { value: `${cur.memory.toFixed(0)}%`, detail: gb(cur.memory) },
    disk: { value: `${cur.disk.toFixed(0)} MB/s`, detail: `read ${(cur.disk * 0.7).toFixed(0)}, write ${(cur.disk * 0.3).toFixed(0)} MB/s` },
    network: { value: `${cur.network.toFixed(0)} MB/s`, detail: `up ${(cur.network * 0.3).toFixed(0)}, down ${(cur.network * 0.7).toFixed(0)} MB/s` },
    ai: { value: `${cur.ai.toFixed(0)} tok/s`, detail: "context 38 percent used" },
  };
}

export const series = writable<Series>(seed());
export const stats = writable<Stats>(fmtStats());

let timer: ReturnType<typeof setInterval> | null = null;

/// Start the ~1 Hz tick (fixture). The real stream is the sidecar seam.
export function startPerf(): void {
  if (timer) return;
  timer = setInterval(() => {
    series.update((s) => {
      const next = {} as Series;
      for (const d of DEVICES) {
        const arr = s[d.key].slice(1);
        arr.push(step(d.key, d.max));
        next[d.key] = arr;
      }
      return next;
    });
    stats.set(fmtStats());
  }, 1000);
}

/// Stop the tick when the Performance tab isn't visible.
export function stopPerf(): void {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
}
