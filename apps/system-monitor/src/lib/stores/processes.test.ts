/// A failed read must be visible as a failed read, and the fixture must not ship.
///
/// `load` deliberately has two failure branches. Under vite it shows a FIXTURE and
/// sets `mocked`, so the frontend is reviewable with no backend. In a real build
/// it must empty the list and set `unavailable`, and the reason is sharper than
/// tidiness: the fixture supplies process ids - 1, 101, 102, 103 - and those ids
/// are the argument to `stop_process`. Id 1 is init. A fixture that reached
/// production would hand a destructive call a real PID.
///
/// The first version of this test asserted the production behaviour without
/// stubbing DEV, so it ran the vite branch and read as a defect. The code was
/// right; the test was.

import { describe, expect, it, beforeEach, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { processes, unavailable, mocked, load } = await import("./processes");
const { get } = await import("svelte/store");

const ONE = [
  { id: 1, name: "claude", status: "Running", cpu: 7.5, memory: 2300000000 },
];

beforeEach(() => {
  invoke.mockReset();
  processes.set([]);
  unavailable.set(false);
  mocked.set(false);
});

describe("load", () => {
  it("takes the list the backend gives it", async () => {
    invoke.mockResolvedValue(ONE);
    await load();
    expect(get(processes)).toHaveLength(1);
    expect(get(unavailable)).toBe(false);
    expect(get(mocked)).toBe(false);
  });

  it("under vite, shows the fixture and says it is a fixture", async () => {
    vi.stubEnv("DEV", true);
    invoke.mockRejectedValue(new Error("no backend"));
    await load();
    expect(get(processes).length).toBeGreaterThan(0);
    expect(get(mocked)).toBe(true);
    expect(get(unavailable)).toBe(false);
    vi.unstubAllEnvs();
  });

  it("in a real build, empties the list and says the read failed", async () => {
    vi.stubEnv("DEV", false);
    invoke.mockRejectedValue(new Error("no backend"));
    await load();
    expect(get(processes)).toHaveLength(0);
    expect(get(unavailable)).toBe(true);
    expect(get(mocked)).toBe(false);
    vi.unstubAllEnvs();
  });

  it("in a real build, does not leave the previous list standing as current", async () => {
    invoke.mockResolvedValue(ONE);
    await load();
    expect(get(processes)).toHaveLength(1);

    vi.stubEnv("DEV", false);
    invoke.mockRejectedValue(new Error("backend went away"));
    await load();
    expect(get(processes)).toHaveLength(0);
    expect(get(unavailable)).toBe(true);
    vi.unstubAllEnvs();
  });
});
