import { describe, it, expect } from "vitest";
import { classifyMark, parseExitCode } from "./block-marks";

describe("classifyMark", () => {
  it("classifies the bare A/C/D marks", () => {
    expect(classifyMark("A")).toBe("prompt-start");
    expect(classifyMark("C")).toBe("exec-start");
    expect(classifyMark("D")).toBe("command-end");
  });

  it("classifies the parameterised forms", () => {
    expect(classifyMark("A;cmdline=…")).toBe("prompt-start");
    expect(classifyMark("C;foo")).toBe("exec-start");
    expect(classifyMark("D;0")).toBe("command-end");
    expect(classifyMark("D;130")).toBe("command-end");
  });

  // The regression: the Arlen shell emits 633;A for prompt-start (not 133;A).
  // The classifier is family-agnostic - the SAME `A` payload arrives whether the
  // OSC was 133 or 633, so registering this on both opcodes routes 633;A. The
  // earlier handler only matched the literal 133;A path and never opened a block.
  it("treats a prompt-start the same regardless of the OSC family (133 vs 633)", () => {
    // Both families deliver "A" as the data payload; one classifier covers both.
    expect(classifyMark("A")).toBe("prompt-start");
  });

  it("ignores marks the chrome does not act on", () => {
    expect(classifyMark("B")).toBeNull(); // prompt-end
    expect(classifyMark("E;ls -la;NONCE")).toBeNull(); // the command line
    expect(classifyMark("P;Cwd=/home")).toBeNull(); // a property mark
    expect(classifyMark("")).toBeNull();
    expect(classifyMark("7;file:///home")).toBeNull(); // not even a letter mark
  });

  it("does not misfire on a letter that merely starts a longer token", () => {
    // Only the exact letter (optionally followed by ';') is a mark; "Abc" is not.
    expect(classifyMark("Abc")).toBeNull();
    expect(classifyMark("Done")).toBeNull();
  });
});

describe("parseExitCode", () => {
  it("returns the exit code from D;<n>", () => {
    expect(parseExitCode("D;0")).toBe(0);
    expect(parseExitCode("D;1")).toBe(1);
    expect(parseExitCode("D;130")).toBe(130);
  });

  it("returns null for a bare D (no exit reported)", () => {
    expect(parseExitCode("D")).toBeNull();
  });

  it("returns null for a malformed or empty exit field", () => {
    expect(parseExitCode("D;")).toBeNull();
    expect(parseExitCode("D;abc")).toBeNull();
  });

  it("reads the exit from the first field only", () => {
    // VS Code's D can carry extra fields after the exit code; take the first.
    expect(parseExitCode("D;0;extra")).toBe(0);
    expect(parseExitCode("D;7;ignored")).toBe(7);
  });
});
