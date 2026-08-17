/// A refused stop must keep the person on the surface that can say so.
///
/// `stopCapture` used to be `invoke(...).catch(() => {})` - the one discarded
/// rejection in this file and the worst one to discard. A refused stop leaves
/// the daemon capturing while the app clears the timers, summarises and walks to
/// the note, so the only thing that changed is that the person believes it
/// stopped and puts the laptop down. The fix is behavioural, not cosmetic: stay
/// here, and say it.
///
/// The dev branch is part of the contract rather than a shortcut. Under vite no
/// command has a host, so a rejection says nothing about the microphone and the
/// stream is a fixture either way; in a real build the same rejection means it
/// may still be live. Both are pinned, because a test written against only one
/// of them passes while the other is wrong.
///
/// No `beforeEach` mock reset here on purpose: a reset makes a rejection set
/// after it escape the code's own catch (isolated while testing the greeter this
/// evening). Each test sets its own implementation and the stores it reads.

import { describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

const invoke = vi.fn((..._args: unknown[]) => Promise.resolve(null as unknown));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { stopCapture, stopFailed, currentId, meeting, liveNotes, liveTranscript } =
  await import("./meeting");

/// Back to the state a live capture would be in, without touching the mock.
function capturing() {
  stopFailed.set(false);
  currentId.set(null);
  meeting.set(null);
  liveNotes.set("what was said");
  liveTranscript.set({ language: "en", segments: [] });
}

describe("stopCapture", () => {
  it("in a real build, a refused stop reports failure and does not open a note", async () => {
    vi.stubEnv("DEV", false);
    capturing();
    invoke.mockImplementation(() => Promise.reject(new Error("the daemon refused")));

    const ok = await stopCapture();

    expect(ok).toBe(false);
    expect(get(stopFailed)).toBe(true);
    // The load-bearing half: navigating away is what makes the microphone
    // invisible again. `currentId` staying null is the app still on this screen.
    expect(get(currentId)).toBeNull();
    expect(get(meeting)).toBeNull();
    vi.unstubAllEnvs();
  });

  it("a stop the daemon accepted goes on to the note", async () => {
    vi.stubEnv("DEV", false);
    capturing();
    invoke.mockImplementation((cmd) =>
      cmd === "meeting_summarize"
        ? Promise.resolve({ summary: "s", decisions: [], actions: [] })
        : Promise.resolve(null),
    );

    const ok = await stopCapture();

    expect(ok).toBe(true);
    expect(get(stopFailed)).toBe(false);
    expect(get(currentId)).toBe("live");
    expect(get(meeting)?.humanNotes).toBe("what was said");
    vi.unstubAllEnvs();
  });

  it("under vite, the same rejection is not read as a live microphone", async () => {
    vi.stubEnv("DEV", true);
    capturing();
    invoke.mockImplementation(() => Promise.reject(new Error("no host")));

    await stopCapture();

    // Nothing here can be evidence about capture, so claiming a failed stop
    // would be the mirror of the defect: a refusal the machine never measured.
    expect(get(stopFailed)).toBe(false);
    vi.unstubAllEnvs();
  });

  it("keeps the human notes when it walks to the note", async () => {
    vi.stubEnv("DEV", false);
    capturing();
    liveNotes.set("only my typing survived");
    invoke.mockImplementation((cmd) =>
      cmd === "meeting_summarize"
        ? Promise.resolve({ summary: "s", decisions: [], actions: [] })
        : Promise.resolve(null),
    );

    await stopCapture();

    expect(get(meeting)?.humanNotes).toBe("only my typing survived");
    vi.unstubAllEnvs();
  });
});
