/// The clock app's state, mirrored from the `org.arlen.Clock1` session daemon
/// through the intended command bridge (clock-app.md §1: the GUI is a VIEW and
/// owns nothing - closing the window changes nothing). That constraint is
/// structural here: the daemon serves ANCHOR TIMESTAMPS (`next_fire_at`,
/// `ends_at`, `started_at`), never counters, and the app derives every display
/// from anchors against the wall clock, ticking only to re-render. All
/// commands are invoke-with-fixture-catch; under vite the fixture stands in
/// and actions apply locally so the whole flow drives.
import { derived, get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One alarm as the daemon serves it. `days` are 0..6 with 0 = Monday (the
/// kit DaysPicker convention); empty means a one-shot alarm.
export interface Alarm {
  id: string;
  /// "HH:MM", the daemon's canonical wall-clock form.
  time: string;
  label: string;
  days: number[];
  enabled: boolean;
  /// Opt-in fire-late-once (§2): ring once after downtime instead of dropping.
  fire_late: boolean;
  /// Epoch ms of the next ring, daemon-computed; null while disabled.
  next_fire_at: number | null;
}

/// One countdown timer. Remaining time derives from `ends_at` (running) or
/// `remaining_ms` (paused snapshot the daemon took).
export interface Timer {
  id: string;
  duration_ms: number;
  ends_at: number | null;
  remaining_ms: number | null;
  paused: boolean;
}

/// The focus session, single by design. `held` names what the enforcement
/// actually suppresses - the surface states it, never asserts it.
export interface FocusSession {
  phase: "focus" | "break";
  round: number;
  rounds: number;
  ends_at: number;
  held: string[];
}

/// Focus configuration (minutes), daemon-persisted.
export interface FocusConfig {
  focus_min: number;
  break_min: number;
  rounds: number;
}

/// The stopwatch: anchors plus daemon-side pause snapshots (clock-app.md §2:
/// CLOCK_MONOTONIC does not pause under s2idle, so the daemon owns pauses).
export interface Stopwatch {
  running: boolean;
  /// Epoch ms the current run started, null when paused or reset.
  started_at: number | null;
  /// Accumulated ms from completed runs.
  accumulated_ms: number;
  /// Lap totals in ms, oldest first.
  laps: number[];
}

/// One world-clock city from the shared offline dataset (a seam; the fixture
/// carries a small list).
export interface WorldCity {
  id: string;
  name: string;
  /// IANA zone name, e.g. "Asia/Tokyo".
  zone: string;
}

/// The daemon state in one read.
export interface ClockState {
  wake_capable: boolean;
  alarms: Alarm[];
  timers: Timer[];
  focus: FocusSession | null;
  focus_config: FocusConfig;
  stopwatch: Stopwatch;
  world: WorldCity[];
}

function fixtureState(now: number): ClockState {
  return {
    wake_capable: true,
    alarms: [
      {
        id: "a1",
        time: "07:00",
        label: "Weekdays",
        days: [0, 1, 2, 3, 4],
        enabled: true,
        fire_late: false,
        next_fire_at: now + 9 * 3600_000,
      },
      {
        id: "a2",
        time: "09:30",
        label: "",
        days: [],
        enabled: false,
        fire_late: true,
        next_fire_at: null,
      },
    ],
    timers: [
      { id: "t1", duration_ms: 25 * 60_000, ends_at: now + 14 * 60_000 + 20_000, remaining_ms: null, paused: false },
    ],
    focus: null,
    focus_config: { focus_min: 25, break_min: 5, rounds: 4 },
    stopwatch: { running: false, started_at: null, accumulated_ms: 0, laps: [] },
    world: [
      { id: "w1", name: "Tokyo", zone: "Asia/Tokyo" },
      { id: "w2", name: "New York", zone: "America/New_York" },
    ],
  };
}

/// The mirrored daemon state, null before the first read settles.
export const clock = writable<ClockState | null>(null);
/// True while the state is the FIXTURE (no daemon under vite).
export const clockMocked = writable(false);

/// A 1 Hz render tick. Surfaces derive displayed remainders from anchors and
/// this tick; it carries no state of its own.
export const tick = writable(Date.now());
let ticker: ReturnType<typeof setInterval> | null = null;
/// Start the render tick (idempotent); the layout owns the lifecycle.
export function startTick(): () => void {
  if (!ticker) ticker = setInterval(() => tick.set(Date.now()), 1000);
  return () => {
    if (ticker) clearInterval(ticker);
    ticker = null;
  };
}

/// DEV: `?nowake` pins the degraded no-wake state so the resting line renders.
const forceNoWake =
  import.meta.env.DEV && typeof location !== "undefined" && new URLSearchParams(location.search).has("nowake");

/// Load the daemon state. Live: `clock_state` over the Clock1 bridge.
export async function loadClock(): Promise<void> {
  try {
    const s = await invoke<ClockState>("clock_state");
    clock.set(forceNoWake ? { ...s, wake_capable: false } : s);
    clockMocked.set(false);
  } catch {
    const s = fixtureState(Date.now());
    clock.set(forceNoWake ? { ...s, wake_capable: false } : s);
    clockMocked.set(true);
  }
}

function patch(fn: (s: ClockState) => ClockState): void {
  clock.update((s) => (s ? fn(s) : s));
}

async function send(cmd: string, args?: Record<string, unknown>): Promise<void> {
  try {
    await invoke(cmd, args);
    await loadClock();
  } catch {
    // Daemon unwired under vite: the local patch stands on the fixture.
  }
}

/// Create or update one alarm; the daemon computes `next_fire_at`.
export async function setAlarm(alarm: Omit<Alarm, "next_fire_at">): Promise<void> {
  patch((s) => {
    const next: Alarm = {
      ...alarm,
      next_fire_at: alarm.enabled ? Date.now() + 8 * 3600_000 : null,
    };
    const at = s.alarms.findIndex((a) => a.id === alarm.id);
    const alarms = at >= 0 ? s.alarms.map((a) => (a.id === alarm.id ? next : a)) : [...s.alarms, next];
    return { ...s, alarms };
  });
  await send("clock_set_alarm", { alarm });
}

/// Arm or disarm one alarm.
export async function toggleAlarm(id: string, enabled: boolean): Promise<void> {
  patch((s) => ({
    ...s,
    alarms: s.alarms.map((a) =>
      a.id === id ? { ...a, enabled, next_fire_at: enabled ? Date.now() + 8 * 3600_000 : null } : a
    ),
  }));
  await send("clock_toggle_alarm", { id, enabled });
}

/// Delete one alarm.
export async function deleteAlarm(id: string): Promise<void> {
  patch((s) => ({ ...s, alarms: s.alarms.filter((a) => a.id !== id) }));
  await send("clock_delete_alarm", { id });
}

/// Start a countdown timer.
export async function startTimer(durationMs: number): Promise<void> {
  patch((s) => ({
    ...s,
    timers: [
      ...s.timers,
      { id: `local-${Date.now()}`, duration_ms: durationMs, ends_at: Date.now() + durationMs, remaining_ms: null, paused: false },
    ],
  }));
  await send("clock_timer_start", { durationMs });
}

/// Pause or resume one timer; the daemon snapshots or re-anchors.
export async function pauseTimer(id: string, paused: boolean): Promise<void> {
  patch((s) => ({
    ...s,
    timers: s.timers.map((ti) => {
      if (ti.id !== id) return ti;
      if (paused) return { ...ti, paused: true, remaining_ms: Math.max(0, (ti.ends_at ?? 0) - Date.now()), ends_at: null };
      return { ...ti, paused: false, ends_at: Date.now() + (ti.remaining_ms ?? 0), remaining_ms: null };
    }),
  }));
  await send("clock_timer_pause", { id, paused });
}

/// Cancel one timer.
export async function cancelTimer(id: string): Promise<void> {
  patch((s) => ({ ...s, timers: s.timers.filter((ti) => ti.id !== id) }));
  await send("clock_timer_cancel", { id });
}

/// Start a focus session from the config. `held` is the daemon's honest list
/// of what the enforcement actually suppresses; the fixture claims exactly the
/// one thing the design mandates.
export async function startFocus(): Promise<void> {
  patch((s) => ({
    ...s,
    focus: {
      phase: "focus",
      round: 1,
      rounds: s.focus_config.rounds,
      ends_at: Date.now() + s.focus_config.focus_min * 60_000,
      held: ["notifications"],
    },
  }));
  await send("clock_focus_start");
}

/// End the session early - always available, fully reversible.
export async function endFocus(): Promise<void> {
  patch((s) => ({ ...s, focus: null }));
  await send("clock_focus_end");
}

/// Persist the focus configuration.
export async function setFocusConfig(config: FocusConfig): Promise<void> {
  patch((s) => ({ ...s, focus_config: config }));
  await send("clock_focus_config", { config });
}

/// Start or resume the stopwatch.
export async function stopwatchStart(): Promise<void> {
  patch((s) => ({ ...s, stopwatch: { ...s.stopwatch, running: true, started_at: Date.now() } }));
  await send("clock_stopwatch_start");
}

/// Pause the stopwatch (daemon-side snapshot).
export async function stopwatchPause(): Promise<void> {
  patch((s) => {
    const sw = s.stopwatch;
    const run = sw.started_at ? Date.now() - sw.started_at : 0;
    return { ...s, stopwatch: { ...sw, running: false, started_at: null, accumulated_ms: sw.accumulated_ms + run } };
  });
  await send("clock_stopwatch_pause");
}

/// Record a lap at the current total.
export async function stopwatchLap(): Promise<void> {
  patch((s) => {
    const sw = s.stopwatch;
    const total = sw.accumulated_ms + (sw.started_at ? Date.now() - sw.started_at : 0);
    return { ...s, stopwatch: { ...sw, laps: [...sw.laps, total] } };
  });
  await send("clock_stopwatch_lap");
}

/// Reset the stopwatch to zero.
export async function stopwatchReset(): Promise<void> {
  patch((s) => ({ ...s, stopwatch: { running: false, started_at: null, accumulated_ms: 0, laps: [] } }));
  await send("clock_stopwatch_reset");
}

/// Add a world-clock city.
export async function addCity(city: WorldCity): Promise<void> {
  patch((s) => (s.world.some((w) => w.id === city.id) ? s : { ...s, world: [...s.world, city] }));
  await send("clock_world_add", { id: city.id });
}

/// Remove a world-clock city.
export async function removeCity(id: string): Promise<void> {
  patch((s) => ({ ...s, world: s.world.filter((w) => w.id !== id) }));
  await send("clock_world_remove", { id });
}

/// The searchable city dataset. The real one is the SHARED offline dataset
/// (clock-app.md §4, a seam); this small list makes the surface drive.
export const CITY_DATASET: WorldCity[] = [
  { id: "w1", name: "Tokyo", zone: "Asia/Tokyo" },
  { id: "w2", name: "New York", zone: "America/New_York" },
  { id: "w3", name: "London", zone: "Europe/London" },
  { id: "w4", name: "Sydney", zone: "Australia/Sydney" },
  { id: "w5", name: "São Paulo", zone: "America/Sao_Paulo" },
  { id: "w6", name: "Nairobi", zone: "Africa/Nairobi" },
  { id: "w7", name: "Innsbruck", zone: "Europe/Vienna" },
  { id: "w8", name: "Los Angeles", zone: "America/Los_Angeles" },
];

/// The stopwatch total at one tick instant.
export function stopwatchTotal(sw: Stopwatch, now: number): number {
  return sw.accumulated_ms + (sw.started_at ? now - sw.started_at : 0);
}

/// A timer's remaining ms at one tick instant.
export function timerRemaining(ti: Timer, now: number): number {
  if (ti.paused) return ti.remaining_ms ?? 0;
  return Math.max(0, (ti.ends_at ?? now) - now);
}
