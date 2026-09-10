// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The two figures the clock re-derives on every tick. Both take `now` as an
// argument rather than reading the machine's, which is what makes them testable
// at all - and both have an edge that shows on screen: a running stopwatch that
// forgets its accumulated time when it is paused and resumed, and a timer that
// counts past zero into negative numbers.
import { describe, it, expect, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => ({}) }));

import { stopwatchTotal, timerRemaining } from "./clock.js";
import type { Stopwatch, Timer } from "./clock.js";

const sw = (accumulated_ms: number, started_at: number | null): Stopwatch =>
  ({ accumulated_ms, started_at }) as unknown as Stopwatch;
const timer = (o: Partial<Timer>): Timer => o as unknown as Timer;

describe("stopwatchTotal", () => {
  it("is what was banked while it is stopped", () => {
    expect(stopwatchTotal(sw(5_000, null), 1_000_000)).toBe(5_000);
  });

  it("adds the running stretch to what was banked", () => {
    // The pause-and-resume case: a stopwatch that dropped `accumulated_ms` would
    // restart from zero on screen every time it was resumed.
    expect(stopwatchTotal(sw(5_000, 1_000_000), 1_002_500)).toBe(7_500);
  });

  it("is zero on a fresh one", () => {
    expect(stopwatchTotal(sw(0, null), 1_000_000)).toBe(0);
  });
});

describe("timerRemaining", () => {
  it("counts down to the end", () => {
    expect(timerRemaining(timer({ paused: false, ends_at: 1_060_000 }), 1_000_000)).toBe(60_000);
  });

  it("stops at zero rather than going negative", () => {
    // A timer past its end must read "00:00", not a growing negative that the
    // formatter would render as something nobody can parse.
    expect(timerRemaining(timer({ paused: false, ends_at: 999_000 }), 1_000_000)).toBe(0);
  });

  it("holds still while paused, whatever the clock does", () => {
    const t = timer({ paused: true, remaining_ms: 42_000, ends_at: 1_000 });
    expect(timerRemaining(t, 1_000_000)).toBe(42_000);
    expect(timerRemaining(t, 9_000_000)).toBe(42_000);
  });

  it("reads a paused timer with nothing banked as zero", () => {
    expect(timerRemaining(timer({ paused: true }), 1_000_000)).toBe(0);
  });

  it("reads a running timer with no end as zero rather than as time left", () => {
    // No `ends_at` is a timer the daemon never armed; showing anything but zero
    // would be inventing a countdown.
    expect(timerRemaining(timer({ paused: false }), 1_000_000)).toBe(0);
  });
});
