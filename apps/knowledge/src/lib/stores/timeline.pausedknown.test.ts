import { describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

// Whether recording is paused is the one claim this app must never make without
// measuring, and the store that carries it defaults to `false` - a value, which a
// Pause button then draws as the position "running". These two cases are what the
// control is allowed to be drawn from.
let answer: () => Promise<unknown> = () => Promise.reject(new Error("no host"));

vi.mock("$lib/tauri", () => ({ tauriAvailable: true }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string) =>
    cmd === "knowledge_timeline_paused" ? answer() : Promise.reject(new Error("not this test")),
}));

const { loadPaused, paused, pausedKnown } = await import("./timeline");

describe("whether the pause state was read", () => {
  it("is known once the daemon has answered", async () => {
    answer = () => Promise.resolve(true);
    await loadPaused();
    expect(get(pausedKnown)).toBe(true);
    expect(get(paused)).toBe(true);
  });

  it("is not known when the read fails, so nothing may render a position", async () => {
    answer = () => Promise.resolve(false);
    await loadPaused();
    expect(get(pausedKnown)).toBe(true);

    answer = () => Promise.reject(new Error("daemon down"));
    await loadPaused();
    expect(get(pausedKnown)).toBe(false);
  });
});
