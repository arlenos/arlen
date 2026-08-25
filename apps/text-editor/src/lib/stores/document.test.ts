/// Text may never appear under a filename that is not its own.
///
/// The three stores here are read together by one branch in the page: an error
/// wins over a document, so a failed open shows the host's message instead of
/// whatever was on screen before. That is the whole contract, and it has two
/// halves that fail in opposite directions - a failure that leaves no error
/// renders the previous file under the new name, and a success that leaves the
/// old error standing hides a file that opened perfectly well.
///
/// `openTarget` is set before the attempt on purpose, so the failure can name
/// the file the person asked for rather than the one still loaded.

import { beforeAll, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

// The store checks this at call time, so the flag has to be true before the
// module is imported. Without it every function returns early and the tests
// would pass by testing nothing.
(globalThis as unknown as { window: unknown }).window = globalThis;
(globalThis as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};

const invoke = vi.fn((..._args: unknown[]) => Promise.resolve(null as unknown));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { openPath, loadInitialFile, openDocument, openError, openTarget } =
  await import("./document");

describe("openPath", () => {
  it("opens a file and clears any earlier failure", async () => {
    openError.set({ problem: "other" });
    invoke.mockImplementation(() =>
      Promise.resolve({ path: "/home/t/notes.md", text: "# Notes" }),
    );

    await openPath("/home/t/notes.md");

    expect(get(openDocument)?.content).toBe("# Notes");
    expect(get(openDocument)?.name).toBe("notes.md");
    // A stale error would hide a file that opened, since the page branches on
    // the error first.
    expect(get(openError)).toBeNull();
  });

  it("reads markdown as prose and everything else as code", async () => {
    invoke.mockImplementation(() => Promise.resolve({ path: "/x/a.md", text: "" }));
    await openPath("/x/a.md");
    expect(get(openDocument)?.type).toBe("markdown");

    invoke.mockImplementation(() => Promise.resolve({ path: "/x/a.rs", text: "" }));
    await openPath("/x/a.rs");
    expect(get(openDocument)?.type).toBe("code");
  });

  it("surfaces a refused open as an error rather than throwing", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("permission denied")));

    // Both callers are user gestures, so this must resolve rather than throw.
    await expect(openPath("/root/secret")).resolves.toBeUndefined();
    // An error the host did not name is recorded as unnamed, and nothing else.
    // It used to carry the host's own words on the grounds that they are the only
    // detail there is - true, and the page drew them bare under a translated
    // heading, so "the only detail there is" was reaching every reader in
    // English. The detail is in the log now.
    expect(get(openError)).toEqual({ problem: "other" });
  });

  it("names the host's own cause rather than pasting its words", async () => {
    // What a real refusal looks like coming back from the command: the payload
    // is an object, not a string with JSON inside it. Getting that wrong sends
    // every named cause down `other`, which reads exactly like it working.
    invoke.mockImplementation(() => Promise.reject({ problem: "not-text" }));
    await openPath("/home/t/photo.jpg");
    expect(get(openError)).toEqual({ problem: "not-text" });

    invoke.mockImplementation(() =>
      Promise.reject({ problem: "unreadable", why: "Permission denied (os error 13)" }),
    );
    await openPath("/root/secret");
    expect(get(openError)).toEqual({
      problem: "unreadable",
      why: "Permission denied (os error 13)",
    });
  });

  it("names the file that was asked for, not the one still loaded", async () => {
    invoke.mockImplementation(() => Promise.resolve({ path: "/x/first.md", text: "one" }));
    await openPath("/x/first.md");

    invoke.mockImplementation(() => Promise.reject(new Error("no such file")));
    await openPath("/x/second.md");

    // The banner names `openTarget`; if it still said `first.md` the message
    // would be about a file that opened fine.
    expect(get(openTarget)).toBe("second.md");
  });
});

describe("loadInitialFile", () => {
  it("does nothing when there is no launch file", async () => {
    openError.set(null);
    openDocument.set(null);
    invoke.mockImplementation(() => Promise.resolve(null));

    await loadInitialFile();

    // No launch file is the ordinary case, not a failure to report.
    expect(get(openError)).toBeNull();
    expect(get(openDocument)).toBeNull();
  });

  it("reports a failure to ask for the launch file", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("host said no")));
    await loadInitialFile();
    // Asking for the launch file has no named causes. That makes it `other`, and
    // `other` says so without quoting the host at the reader.
    expect(get(openError)).toEqual({ problem: "other" });
  });
});

describe("saveProblemKey", () => {
  /// Imported inside the block for the same reason as the stores above: the
  /// module reads the Tauri flag at import time.
  let saveProblemKey: (e: unknown) => string;
  beforeAll(async () => {
    saveProblemKey = (await import("./document")).saveProblemKey;
  });

  /// The host answers a refused save with a tagged problem. Each tag must reach
  /// its own sentence, because the fallback is the vague one and a tag quietly
  /// falling through to it looks exactly like the code working.
  it("names every tag the host can return", () => {
    expect(saveProblemKey({ problem: "not-absolute" })).toBe("te.save.notAbsolute");
    expect(saveProblemKey({ problem: "no-parent" })).toBe("te.save.noParent");
    expect(saveProblemKey({ problem: "unwritable", why: "Permission denied" })).toBe(
      "te.save.unwritable",
    );
  });

  /// A Tauri error arrives as an object on one path and as a string with the JSON
  /// inside it on another. Guessing one sends every named cause down the vague
  /// branch, which is the failure this shares with the open decoder beside it.
  it("reads the tag out of a stringified error too", () => {
    expect(saveProblemKey('invoke error: {"problem":"unwritable","why":"nope"}')).toBe(
      "te.save.unwritable",
    );
  });

  /// The floor: nothing untranslated escapes. Whatever comes back, the page shows
  /// a key it has a sentence for, never the host's own words.
  it("falls back to a sentence rather than showing what it got", () => {
    for (const e of ["Permission denied (os error 13)", null, undefined, 42, {}]) {
      expect(saveProblemKey(e)).toMatch(/^te\.save\./);
    }
  });
});
