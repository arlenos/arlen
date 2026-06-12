/// The controller's thumbnail cache: dedup of in-flight requests,
/// null-result caching, LRU bound and ordering, mtime-sensitive keys,
/// refresh invalidation, and the no-adapter no-op.
import { get } from "svelte/store";
import { describe, expect, it, vi } from "vitest";
import { createBrowserState } from "./controller";
import type { BrowserAdapter, FileEntry } from "./types";

const entry = (name: string, mtime = 100): FileEntry => ({
  name,
  kind: "file",
  size: 1,
  modified_unix: mtime,
  is_hidden: false,
  readonly: false,
  symlink_target: null,
});

/// An adapter whose thumbnail promises resolve only when the test
/// says so, to hold requests in flight.
function deferredAdapter(entries: FileEntry[]) {
  const pending: { path: string; resolve: (url: string | null) => void }[] = [];
  const adapter: BrowserAdapter = {
    list: () => Promise.resolve(entries),
    thumbnail: vi.fn(
      (path: string) =>
        new Promise<string | null>((resolve) => {
          pending.push({ path, resolve });
        }),
    ),
  };
  return { adapter, pending };
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe("controller thumbnail cache", () => {
  it("dedupes requests while one is in flight and caches the result", async () => {
    const e = entry("a.png");
    const { adapter, pending } = deferredAdapter([e]);
    const c = createBrowserState(adapter, { initial: "/home" });

    c.requestThumbnail(e);
    c.requestThumbnail(e);
    c.requestThumbnail(e);
    expect(adapter.thumbnail).toHaveBeenCalledTimes(1);
    expect(pending[0]?.path).toBe("/home/a.png");

    pending[0]!.resolve("data:image/png;base64,AAA");
    await flush();
    expect(get(c.thumbnails).get(c.thumbnailKeyFor(e))).toBe(
      "data:image/png;base64,AAA",
    );

    // A settled key never re-invokes.
    c.requestThumbnail(e);
    expect(adapter.thumbnail).toHaveBeenCalledTimes(1);
  });

  it("caches a null result so a broken image is asked once", async () => {
    const e = entry("broken.png");
    const { adapter, pending } = deferredAdapter([e]);
    const c = createBrowserState(adapter, { initial: "/home" });

    c.requestThumbnail(e);
    pending[0]!.resolve(null);
    await flush();
    expect(get(c.thumbnails).has(c.thumbnailKeyFor(e))).toBe(true);
    expect(get(c.thumbnails).get(c.thumbnailKeyFor(e))).toBe(null);

    c.requestThumbnail(e);
    expect(adapter.thumbnail).toHaveBeenCalledTimes(1);
  });

  it("a rejected promise settles as null instead of staying in flight", async () => {
    const e = entry("flaky.png");
    const adapter: BrowserAdapter = {
      list: () => Promise.resolve([e]),
      thumbnail: vi.fn(() => Promise.reject(new Error("io"))),
    };
    const c = createBrowserState(adapter, { initial: "/home" });
    c.requestThumbnail(e);
    await flush();
    expect(get(c.thumbnails).get(c.thumbnailKeyFor(e))).toBe(null);
    c.requestThumbnail(e);
    expect(adapter.thumbnail).toHaveBeenCalledTimes(1);
  });

  it("evicts the least recently requested entry past the bound", async () => {
    const many = Array.from({ length: 301 }, (_, i) => entry(`p${i}.png`));
    const adapter: BrowserAdapter = {
      list: () => Promise.resolve(many),
      thumbnail: (path: string) => Promise.resolve(`url:${path}`),
    };
    const c = createBrowserState(adapter, { initial: "/home" });

    for (const e of many.slice(0, 300)) c.requestThumbnail(e);
    await flush();
    expect(get(c.thumbnails).size).toBe(300);

    // Touch the oldest so the second-oldest is evicted instead.
    c.requestThumbnail(many[0]!);
    c.requestThumbnail(many[300]!);
    await flush();
    const map = get(c.thumbnails);
    expect(map.size).toBe(300);
    expect(map.has(c.thumbnailKeyFor(many[0]!))).toBe(true);
    expect(map.has(c.thumbnailKeyFor(many[1]!))).toBe(false);
    expect(map.has(c.thumbnailKeyFor(many[300]!))).toBe(true);
  });

  it("keys change with mtime, so an edited file refetches", async () => {
    const before = entry("a.png", 100);
    const after = entry("a.png", 200);
    const { adapter, pending } = deferredAdapter([after]);
    const c = createBrowserState(adapter, { initial: "/home" });

    c.requestThumbnail(before);
    pending[0]!.resolve("url:old");
    await flush();

    c.requestThumbnail(after);
    expect(adapter.thumbnail).toHaveBeenCalledTimes(2);
    expect(c.thumbnailKeyFor(before)).not.toBe(c.thumbnailKeyFor(after));
  });

  it("refresh clears the cache like the listing cache", async () => {
    const e = entry("a.png");
    const { adapter, pending } = deferredAdapter([e]);
    const c = createBrowserState(adapter, { initial: "/home" });

    c.requestThumbnail(e);
    pending[0]!.resolve("url:a");
    await flush();
    expect(get(c.thumbnails).size).toBe(1);

    await c.refresh();
    expect(get(c.thumbnails).size).toBe(0);
  });

  it("is a no-op without an adapter method and for non-files", () => {
    const bare: BrowserAdapter = { list: () => Promise.resolve([]) };
    const c = createBrowserState(bare, { initial: "/home" });
    expect(c.hasThumbnails).toBe(false);
    c.requestThumbnail(entry("a.png"));
    expect(get(c.thumbnails).size).toBe(0);

    const { adapter } = deferredAdapter([]);
    const c2 = createBrowserState(adapter, { initial: "/home" });
    expect(c2.hasThumbnails).toBe(true);
    c2.requestThumbnail({ ...entry("dir"), kind: "directory" });
    expect(adapter.thumbnail).not.toHaveBeenCalled();
  });
});
