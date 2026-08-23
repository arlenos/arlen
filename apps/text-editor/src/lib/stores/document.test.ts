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

import { describe, expect, it, vi } from "vitest";
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
    openError.set({ problem: "other", reason: "something older went wrong" });
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
    // An error the host did not name keeps its own words, which is the only
    // detail there is for it.
    expect(get(openError)).toEqual({
      problem: "other",
      reason: expect.stringContaining("permission denied"),
    });
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
    // Asking for the launch file has no named causes - whatever went wrong there
    // is the host's own words and there is nothing else to say about it.
    expect(get(openError)).toEqual({
      problem: "other",
      reason: expect.stringContaining("host said no"),
    });
  });
});
