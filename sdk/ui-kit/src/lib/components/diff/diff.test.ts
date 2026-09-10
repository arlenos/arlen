// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The diff parser had no test, and its consumer is the harness gate card - the
// surface where a person is shown what an AI proposes to change and presses
// yes or no. A parser that drops a file, mislabels a deletion as an addition or
// silently swallows a hunk changes what that person is agreeing to. Every case
// below is a shape real tooling emits.
import { describe, it, expect } from "vitest";
import { parseUnifiedDiff, diffTotals } from "./diff.js";

describe("parseUnifiedDiff", () => {
  it("reads a git diff of one modified file", () => {
    const files = parseUnifiedDiff(
      [
        "diff --git a/src/main.rs b/src/main.rs",
        "index 1234567..89abcde 100644",
        "--- a/src/main.rs",
        "+++ b/src/main.rs",
        "@@ -1,3 +1,4 @@",
        " fn main() {",
        "-    old();",
        "+    fresh();",
        "+    more();",
        " }",
      ].join("\n"),
    );
    expect(files).toHaveLength(1);
    expect(files[0].path).toBe("src/main.rs");
    expect(files[0].status).toBe("modified");
    expect(files[0].additions).toBe(2);
    expect(files[0].deletions).toBe(1);
    expect(files[0].hunks).toHaveLength(1);
    expect(files[0].hunks[0].header).toBe("@@ -1,3 +1,4 @@");
    // The leading marker is stripped into `kind`, and a context line keeps its
    // text without the space that carried it.
    expect(files[0].hunks[0].lines).toEqual([
      { kind: "context", text: "fn main() {" },
      { kind: "del", text: "    old();" },
      { kind: "add", text: "    fresh();" },
      { kind: "add", text: "    more();" },
      { kind: "context", text: "}" },
    ]);
  });

  it("keeps several files apart", () => {
    const files = parseUnifiedDiff(
      [
        "diff --git a/one.txt b/one.txt",
        "--- a/one.txt",
        "+++ b/one.txt",
        "@@ -1 +1 @@",
        "-a",
        "+b",
        "diff --git a/two.txt b/two.txt",
        "--- a/two.txt",
        "+++ b/two.txt",
        "@@ -1 +1 @@",
        "-c",
        "+d",
      ].join("\n"),
    );
    expect(files.map((f) => f.path)).toEqual(["one.txt", "two.txt"]);
    expect(files.every((f) => f.additions === 1 && f.deletions === 1)).toBe(true);
  });

  it("names an added, a deleted and a renamed file as such", () => {
    const added = parseUnifiedDiff(
      [
        "diff --git a/new.txt b/new.txt",
        "new file mode 100644",
        "--- /dev/null",
        "+++ b/new.txt",
        "@@ -0,0 +1 @@",
        "+hello",
      ].join("\n"),
    );
    expect(added[0]).toMatchObject({ path: "new.txt", status: "added", additions: 1 });

    const deleted = parseUnifiedDiff(
      [
        "diff --git a/gone.txt b/gone.txt",
        "deleted file mode 100644",
        "--- a/gone.txt",
        "+++ /dev/null",
        "@@ -1 +0,0 @@",
        "-bye",
      ].join("\n"),
    );
    expect(deleted[0]).toMatchObject({ path: "gone.txt", status: "deleted", deletions: 1 });

    const renamed = parseUnifiedDiff(
      [
        "diff --git a/old/name.txt b/new/name.txt",
        "similarity index 98%",
        "rename from old/name.txt",
        "rename to new/name.txt",
      ].join("\n"),
    );
    expect(renamed[0]).toMatchObject({
      path: "new/name.txt",
      status: "renamed",
      oldPath: "old/name.txt",
    });
  });

  it("keeps a rename with no hunks, and drops a modified file with none", () => {
    // A pure rename is a real change with nothing inside it; a "modified" file
    // that produced no hunk is a header the parser could make nothing of, and
    // showing it would claim a change nobody can see.
    const both = parseUnifiedDiff(
      [
        "diff --git a/a.txt b/b.txt",
        "rename from a.txt",
        "rename to b.txt",
        "diff --git a/quiet.txt b/quiet.txt",
        "index 111..222 100644",
      ].join("\n"),
    );
    expect(both.map((f) => f.path)).toEqual(["b.txt"]);
  });

  it("reads a plain diff that never says `diff --git`", () => {
    const files = parseUnifiedDiff(
      ["--- a/notes.md", "+++ b/notes.md", "@@ -1 +1 @@", "-was", "+is"].join("\n"),
    );
    expect(files).toHaveLength(1);
    expect(files[0].path).toBe("notes.md");
    expect(files[0].additions).toBe(1);
  });

  it("skips the no-newline marker instead of counting it as a line", () => {
    const files = parseUnifiedDiff(
      [
        "--- a/x",
        "+++ b/x",
        "@@ -1 +1 @@",
        "-one",
        "\\ No newline at end of file",
        "+two",
      ].join("\n"),
    );
    expect(files[0].hunks[0].lines).toEqual([
      { kind: "del", text: "one" },
      { kind: "add", text: "two" },
    ]);
  });

  it("counts each hunk of a file into one total", () => {
    const files = parseUnifiedDiff(
      [
        "--- a/x",
        "+++ b/x",
        "@@ -1 +1 @@",
        "+a",
        "@@ -10 +10 @@",
        "+b",
        "-c",
      ].join("\n"),
    );
    expect(files[0].hunks).toHaveLength(2);
    expect(files[0].additions).toBe(2);
    expect(files[0].deletions).toBe(1);
  });

  it("answers nothing for an empty string", () => {
    expect(parseUnifiedDiff("")).toEqual([]);
  });
});

describe("diffTotals", () => {
  it("adds up a change set", () => {
    const files = parseUnifiedDiff(
      [
        "diff --git a/one b/one",
        "--- a/one",
        "+++ b/one",
        "@@ -1 +1,2 @@",
        "+a",
        "+b",
        "-c",
        "diff --git a/two b/two",
        "--- a/two",
        "+++ b/two",
        "@@ -1 +1 @@",
        "+d",
      ].join("\n"),
    );
    expect(diffTotals(files)).toEqual({ additions: 3, deletions: 1 });
  });

  it("is zero for no files", () => {
    expect(diffTotals([])).toEqual({ additions: 0, deletions: 0 });
  });
});
