import { describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

// The host is absent, and every fixture branch in this store used to key on
// `import.meta.env.DEV` instead. That answered a question about the build where
// the branch needed one about the backend, and it was wrong in both directions:
// under `tauri dev` a real failure showed invented meetings, and a headless
// release render reported that meetings were unavailable when nothing had failed.
//
// The live-transcript branch made the first direction concrete. Its own comment
// records that it "streamed invented sentences about a KG lens into a real
// meeting's live transcript while the person watched it fill in" - a DEV build
// with a working backend is exactly the session where that happens.
vi.mock("$lib/tauri", () => ({ tauriAvailable: false }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.reject(new Error("no host")),
}));

const { loadMeetings, meetings, meetingsMocked, meetingsUnavailable } = await import("./meeting");

describe("meetings with no host", () => {
  it("shows the labelled sample rather than reporting meetings unavailable", async () => {
    await loadMeetings();
    expect(get(meetingsMocked)).toBe(true);
    expect(get(meetingsUnavailable)).toBe(false);
    expect(get(meetings).length).toBeGreaterThan(0);
  });
});
