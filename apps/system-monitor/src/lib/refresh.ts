/// How often the tool re-reads the machine (system-monitor-plan.md (a), "a
/// global refresh-rate control", beside freeze-the-refresh).
///
/// One rate for both pollers. The process list and the Performance tab sampled
/// at 2s and 1s from two independent `setInterval`s, so "how current is this"
/// had two different answers on two tabs of the same window and no way to change
/// either. The btop and htop users the plan names expect to set this.
///
/// WHY A FLOOR AND A CEILING. This is the one app where the poll cost lands on
/// the machine the user opened it to relieve: each tick walks `/proc` for every
/// process. A stored `0` would mean `setInterval(fn, 0)`, which is a busy loop
/// inside the process monitor - the tool becoming the top row of its own list.
/// So a persisted value is validated rather than trusted, and anything outside
/// the offered set falls back to the default.

/// The offered rates, fastest first. Milliseconds.
///
/// 500ms is the floor deliberately: below that a `/proc` walk of a few hundred
/// processes stops being free, and the numbers are noise rather than detail.

import { writable } from "svelte/store";

export const RATES = [500, 1000, 2000, 5000, 10000] as const;

/// The default: fast enough to catch a spike, slow enough to be free.
export const DEFAULT_RATE = 2000;

/// Where the choice is remembered between windows.
export const RATE_KEY = "arlen.system-monitor.refreshMs";

/// Turn a remembered value into a rate we will actually run.
///
/// Fails to the default rather than to whatever was stored: a corrupt or
/// hand-edited `localStorage` entry must not be able to set a zero interval, and
/// a rate we no longer offer (say a 100ms one from an older build) should land
/// on something sane rather than persist forever.
export function parseRate(stored: string | null | undefined): number {
  if (stored == null) return DEFAULT_RATE;
  const n = Number(stored);
  if (!Number.isFinite(n)) return DEFAULT_RATE;
  return (RATES as readonly number[]).includes(n) ? n : DEFAULT_RATE;
}

/// The label for a rate, in plain words rather than a raw millisecond count.
export function rateLabel(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${ms / 1000} s`;
}


/// The live rate, shared by the process poller and the Performance sampler.
export const refreshMs = writable<number>(
  parseRate(typeof localStorage === "undefined" ? null : localStorage.getItem(RATE_KEY)),
);

/// Change the rate and remember it.
///
/// Validated on the way in as well as on the way out: the control is not the
/// only possible caller, and a rate that reached the store unchecked would run
/// before the next window ever revalidated it.
export function setRefreshMs(ms: number): void {
  const rate = parseRate(String(ms));
  refreshMs.set(rate);
  try {
    localStorage.setItem(RATE_KEY, String(rate));
  } catch {
    // A private window with storage denied still gets a working control for
    // this session; only the memory of it is lost.
  }
}
