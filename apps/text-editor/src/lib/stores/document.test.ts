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
    openError.set("something older went wrong");
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
    expect(get(openError)).toContain("permission denied");
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
    expect(get(openError)).toContain("host said no");
  });
});
