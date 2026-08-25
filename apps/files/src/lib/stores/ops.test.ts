/// The words a refused file operation reaches the window with.
///
/// Two properties, and the second is the one that used to be prose. Every tagged
/// problem must reach its own sentence, AND `already-exists` must be readable as
/// a TAG - the conflict dialog is raised off it, so it is behaviour rather than a
/// message, and it used to hang on `String(e).match(/already exists/)`.
import { describe, expect, it } from "vitest";
import { opProblemKey, problemBag } from "./ops";

describe("opProblemKey", () => {
  it("names every problem the host can return", () => {
    expect(opProblemKey({ problem: "already-exists", name: "notes.md" })).toBe("f.op.exists");
    expect(opProblemKey({ problem: "invalid-name", name: ".." })).toBe("f.op.badName");
    expect(opProblemKey({ problem: "partial", why: "…" })).toBe("f.op.partial");
    expect(opProblemKey({ problem: "io", why: "Permission denied" })).toBe("f.op.refused");
  });

  /// A malformed call is not something the person did, so it says the vague true
  /// thing rather than naming a missing argument to somebody who pressed Rename.
  it("keeps a bad request out of the person's way", () => {
    expect(opProblemKey({ problem: "bad-request", why: "rename needs a source" })).toBe(
      "f.op.failed",
    );
  });

  /// The floor: whatever arrives, the bar shows a key it has a sentence for.
  it("never returns the host's own words", () => {
    for (const e of ["destination already exists: notes.md", null, undefined, 42, {}]) {
      expect(opProblemKey(e)).toMatch(/^f\.op\./);
    }
  });
});

describe("problemBag", () => {
  /// A Tauri error arrives as an object on one path and as a string with the JSON
  /// inside it on another. The conflict dialog depends on reading BOTH: accepting
  /// only one would turn a choice into a red bar on whichever path it missed.
  it("reads the tag out of either shape", () => {
    expect(problemBag({ problem: "already-exists", name: "a" })?.problem).toBe("already-exists");
    expect(
      problemBag('invoke error: {"problem":"already-exists","name":"a"}')?.name,
    ).toBe("a");
    expect(problemBag("not json at all")).toBeNull();
  });
});
