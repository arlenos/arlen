import { describe, expect, it, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

// No Tauri host, which is the whole subject: the search store used to invoke
// regardless, catch the resulting TypeError, and set `searchFailed` - so a
// browser tab or a screenshot run reported that the user's search had FAILED,
// in a window that never had a backend to search with.
vi.mock("$lib/tauri", () => ({ tauriAvailable: false }));

const { runSearch, searchFailed, searchUnavailable, searchResults, searchQuery } = await import("./search");

describe("search with no host", () => {
  beforeEach(() => {
    searchFailed.set(false);
    searchUnavailable.set(false);
    // An empty query returns before any of this, which is correct behaviour and
    // would have made both cases pass for the wrong reason.
    searchQuery.set("report");
  });

  it("reports unavailable rather than failed", async () => {
    await runSearch("/home/someone");
    expect(get(searchUnavailable)).toBe(true);
    expect(get(searchFailed)).toBe(false);
  });

  it("still returns an empty list rather than leaving stale hits on screen", async () => {
    // `null` means "no search is active" to the results view, so a previous
    // query's hits would keep rendering under the new one.
    searchResults.set([{ path: "/old/hit" } as never]);
    await runSearch("/home/someone");
    expect(get(searchResults)).toEqual([]);
  });
});
