import { describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

// No Tauri host, which is the case the branch is really about.
//
// This store used to ask `import.meta.env.DEV` here. That is a question about
// the BUILD, and the branch needs a question about the BACKEND, so it answered
// wrongly in both directions: under `tauri dev` a genuinely failed read took the
// fixture path and the landing view showed a week of invented history as fact,
// while a release build rendered headlessly - every screenshot drive - is not a
// DEV build and has no backend either, so it reported that the timeline was
// unavailable when nothing had gone wrong.
vi.mock("$lib/tauri", () => ({ tauriAvailable: false }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.reject(new Error("no host")),
}));

const { loadTimeline, timelineMocked, timelineUnavailable, days } = await import("./timeline");

describe("the timeline with no host", () => {
  it("shows the labelled sample rather than claiming the timeline is unavailable", async () => {
    await loadTimeline();
    expect(get(timelineMocked)).toBe(true);
    expect(get(timelineUnavailable)).toBe(false);
    expect(get(days)?.length).toBeGreaterThan(0);
  });
});
