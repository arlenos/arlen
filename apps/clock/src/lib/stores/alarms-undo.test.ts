// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Removing an alarm asks nothing because it can come back: the store keeps the
// removed alarm while the way back is open and closes the way on the next act.
// Without a host the store serves the fixture and patches locally, which is the
// optimistic path every act takes before the daemon answers.
import { describe, it, expect, vi } from "vitest";
import { get } from "svelte/store";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => { throw new Error("no host"); } }));

import { clock, loadClock, deleteAlarm, undoRemove, toggleAlarm, removedAlarm } from "./clock.js";

const ids = () => (get(clock)?.alarms ?? []).map((a) => a.id);

describe("removing an alarm", () => {
  it("keeps the removed alarm and puts it back on undo", async () => {
    await loadClock();
    expect(ids()).toEqual(["a1", "a2"]);
    await deleteAlarm("a1");
    expect(ids()).toEqual(["a2"]);
    expect(get(removedAlarm)?.id).toBe("a1");
    await undoRemove();
    expect(ids().sort()).toEqual(["a1", "a2"]);
    const back = get(clock)?.alarms.find((a) => a.id === "a1");
    expect(back?.time).toBe("07:00");
    expect(back?.days).toEqual([0, 1, 2, 3, 4]);
    expect(back?.enabled).toBe(true);
    expect(get(removedAlarm)).toBeNull();
  });

  it("closes the way back on the next act", async () => {
    await loadClock();
    await deleteAlarm("a2");
    expect(get(removedAlarm)?.id).toBe("a2");
    await toggleAlarm("a1", false);
    expect(get(removedAlarm)).toBeNull();
    await undoRemove();
    expect(ids()).toEqual(["a1"]);
  });
});
