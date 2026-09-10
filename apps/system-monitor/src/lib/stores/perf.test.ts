// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The graph's ceiling, which decides whether a chart tells the truth about a
// machine. Fixed at a floor, a burst of 400 MB/s draws flat against 50 and reads
// as "nothing happening"; scaled to the peak always, an idle machine draws its
// own noise as mountains. This is the one function that picks, and the screenshot
// probes cannot see a wrong axis - the chart looks perfectly well drawn either
// way.
import { describe, it, expect, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => ({}) }));

import { axisMax, sampleTick } from "./perf.js";

describe("axisMax", () => {
  it("keeps the floor for an idle machine", () => {
    expect(axisMax([0, 0, 1, 2], 50)).toBe(50);
    // Exactly at the floor is still the floor: a ceiling equal to the peak would
    // draw the line along the top edge.
    expect(axisMax([50], 50)).toBe(50);
  });

  it("keeps the floor before the first tick", () => {
    expect(axisMax([], 50)).toBe(50);
  });

  it("rises above a peak that beats the floor, with headroom", () => {
    // Ten percent, so the highest point is not drawn on the frame.
    expect(axisMax([400], 50)).toBeCloseTo(440);
    expect(axisMax([10, 200, 30], 50)).toBeCloseTo(220);
  });

  it("follows the largest value, not the latest", () => {
    // The series is a rolling window; a burst that has scrolled to the middle
    // still sets the scale the rest is drawn against.
    expect(axisMax([300, 5, 5, 5], 50)).toBeCloseTo(330);
  });

  it("never returns something a value could not be drawn inside", () => {
    for (const values of [[0], [1, 2, 3], [999], [0.5, 0.25]]) {
      const max = axisMax(values, 50);
      for (const v of values) expect(v).toBeLessThanOrEqual(max);
    }
  });
});

describe("sampleTick", () => {
  it("is deterministic, so two renders of the sample agree", () => {
    expect(sampleTick(7)).toEqual(sampleTick(7));
    expect(sampleTick(7).cpuPct).not.toBe(sampleTick(8).cpuPct);
  });

  it("carries measured rates, never a first-tick zero", () => {
    const t = sampleTick(0);
    expect(t.ratesReady).toBe(true);
    expect(t.cores).toHaveLength(8);
    expect(t.diskReadMbs).not.toBeNull();
    expect(t.memPct).toBeGreaterThan(50);
    expect(t.memPct).toBeLessThan(60);
  });
});
