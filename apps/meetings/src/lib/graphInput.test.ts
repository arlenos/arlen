import { describe, it, expect } from "vitest";
import { isCapturing, capturePresence, LIVE } from "./graphInput";

describe("isCapturing", () => {
  it("is on only once a capture has actually started", () => {
    expect(isCapturing(true)).toBe(true);
  });

  /// THE DEFECT THIS REPLACED. The first version read `!unavailable &&
  /// !stopFailed`, and both of those are false in the moment between pressing
  /// Start and the host answering - so on a machine with no ASR engine the
  /// window told the graph a meeting was happening and then correctly said
  /// nothing was being captured. A drive watching the wire caught it.
  it("is off before the host has answered, so a refused start claims nothing", () => {
    expect(isCapturing(false)).toBe(false);
  });
});

describe("capturePresence", () => {
  it("says a meeting is happening, and whether it is being transcribed", () => {
    expect(capturePresence(true, null, true)).toEqual({
      activity: "meeting",
      subject: LIVE,
      metadata: { transcribing: "true" },
      auto_clear: "on-blur",
    });
  });

  it("names the meeting once the graph has filed it", () => {
    expect(capturePresence(true, "m-42", false)?.subject).toBe("m-42");
  });

  it("separates the two consents rather than folding them together", () => {
    expect(capturePresence(true, null, false)?.metadata?.transcribing).toBe("false");
  });

  it("claims nothing when nothing is being captured", () => {
    expect(capturePresence(false, "m-42", true)).toBeNull();
  });
});
