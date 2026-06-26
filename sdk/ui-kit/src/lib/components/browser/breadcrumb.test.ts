import { describe, it, expect } from "vitest";
import { breadcrumb, isVirtualLocation, locationCrumbs } from "./breadcrumb";

describe("breadcrumb", () => {
  it("decomposes an absolute path into a navigable hierarchy", () => {
    expect(breadcrumb("/home/x/p")).toEqual([
      { name: "/", path: "/" },
      { name: "home", path: "/home" },
      { name: "x", path: "/home/x" },
      { name: "p", path: "/home/x/p" },
    ]);
  });

  it("ignores . and .. segments", () => {
    expect(breadcrumb("/a/./b/../c")).toEqual([
      { name: "/", path: "/" },
      { name: "a", path: "/a" },
      { name: "b", path: "/a/b" },
      { name: "c", path: "/a/b/c" },
    ]);
  });
});

describe("isVirtualLocation", () => {
  it("treats an absolute path as a real location", () => {
    expect(isVirtualLocation("/home/x")).toBe(false);
    expect(isVirtualLocation("/")).toBe(false);
  });

  it("treats a non-slash key as a virtual location", () => {
    expect(isVirtualLocation("recent")).toBe(true);
    expect(isVirtualLocation("trash")).toBe(true);
    expect(isVirtualLocation("project:abc-123")).toBe(true);
    expect(isVirtualLocation("search:main.rs")).toBe(true);
  });

  it("treats the empty string as neither (no location)", () => {
    expect(isVirtualLocation("")).toBe(false);
  });
});

describe("locationCrumbs", () => {
  it("gives a real path its full navigable hierarchy, ignoring the label", () => {
    expect(locationCrumbs("/home/x/p", "unused")).toEqual([
      { name: "/", path: "/" },
      { name: "home", path: "/home" },
      { name: "x", path: "/home/x" },
      { name: "p", path: "/home/x/p" },
    ]);
  });

  it("gives a virtual location ONE non-navigable name crumb with the host label", () => {
    // The label is the host's (translated, or a project's own name); the crumb path
    // is the location itself so a click round-trips it back through the adapter.
    expect(locationCrumbs("recent", "Recent")).toEqual([{ name: "Recent", path: "recent" }]);
    expect(locationCrumbs("project:abc", "Arlen")).toEqual([
      { name: "Arlen", path: "project:abc" },
    ]);
    expect(locationCrumbs("search:notes", "Search: notes")).toEqual([
      { name: "Search: notes", path: "search:notes" },
    ]);
  });

  it("never splits a virtual location on its inner characters (one crumb only)", () => {
    // A `project:a/b` key (a colon, a slash) is still a single place, not a path.
    expect(locationCrumbs("project:a/b", "Proj")).toEqual([{ name: "Proj", path: "project:a/b" }]);
  });
});
