import { describe, it, expect } from "vitest";
import { shellPresence } from "./graphInput";

describe("shellPresence", () => {
  it("says somebody is at a shell, and where", () => {
    expect(shellPresence("/home/tim/src", "running")).toEqual({
      activity: "shell",
      subject: "/home/tim/src",
      metadata: { status: "running" },
      auto_clear: "on-blur",
    });
  });

  it("claims nothing with no session", () => {
    expect(shellPresence(null, "running")).toBeNull();
  });

  it("omits a status it does not have", () => {
    expect(shellPresence("/home/tim", "")?.metadata).toBeUndefined();
  });

  /// THE LOAD-BEARING ONE. A shell history is the most revealing record a
  /// desktop keeps; the subject is a directory and must stay one. If somebody
  /// ever widens this to the command line, that is a consent decision and this
  /// test is where it has to be argued rather than slipped through.
  it("publishes a directory and never a command", () => {
    const p = shellPresence("/home/tim/src", "running");
    expect(JSON.stringify(p)).not.toMatch(/command|argv|line/i);
    expect(p?.subject).toBe("/home/tim/src");
  });
});
