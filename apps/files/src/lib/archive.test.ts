import { describe, it, expect } from "vitest";
import {
  isArchiveName,
  splitArchivePath,
  archiveListing,
  sortEntries,
  type ArchiveEntry,
} from "./archive";

describe("isArchiveName", () => {
  it("matches the supported archive extensions, case-insensitively", () => {
    for (const n of ["a.zip", "A.ZIP", "x.tar", "x.tar.gz", "x.tgz"]) {
      expect(isArchiveName(n)).toBe(true);
    }
  });
  it("rejects non-archives", () => {
    for (const n of ["a.txt", "zip", "tarball", "x.gz", "x.tar.bz2"]) {
      expect(isArchiveName(n)).toBe(false);
    }
  });
});

describe("splitArchivePath", () => {
  it("splits at the archive file, returning the inner path", () => {
    expect(splitArchivePath("/home/u/a.zip")).toEqual({
      archive: "/home/u/a.zip",
      inner: "",
    });
    expect(splitArchivePath("/home/u/a.zip/sub/x.txt")).toEqual({
      archive: "/home/u/a.zip",
      inner: "sub/x.txt",
    });
  });
  it("is null when no component is an archive", () => {
    expect(splitArchivePath("/home/u/docs/x.txt")).toBeNull();
  });
});

const ENTRIES: ArchiveEntry[] = [
  { path: "readme.txt", size: 10, is_dir: false },
  { path: "sub/", size: 0, is_dir: true },
  { path: "sub/a.txt", size: 3, is_dir: false },
  { path: "sub/deep/b.txt", size: 4, is_dir: false },
  { path: "only/deep/c.txt", size: 5, is_dir: false },
];

describe("archiveListing", () => {
  it("projects the root: direct files + synthesized top-level dirs", () => {
    const names = archiveListing(ENTRIES, "").map((e) => e.name).sort();
    // `sub` (explicit dir) and `only` (synthesized from only/deep/c.txt) and readme.txt.
    expect(names).toEqual(["only", "readme.txt", "sub"]);
    const sub = archiveListing(ENTRIES, "").find((e) => e.name === "sub")!;
    expect(sub.kind).toBe("directory");
    expect(sub.readonly).toBe(true);
  });
  it("projects a sub-path to its direct children", () => {
    const sub = archiveListing(ENTRIES, "sub").map((e) => e.name).sort();
    // a.txt (direct) + deep (synthesized from sub/deep/b.txt).
    expect(sub).toEqual(["a.txt", "deep"]);
  });
  it("does not synthesize a dir that already has an explicit entry", () => {
    const root = archiveListing(ENTRIES, "");
    expect(root.filter((e) => e.name === "sub")).toHaveLength(1);
  });
});

describe("sortEntries", () => {
  const rows = archiveListing(ENTRIES, "");
  it("puts folders first, then name-ascending", () => {
    const out = sortEntries(rows, { key: "name", foldersFirst: true, ascending: true });
    expect(out.map((e) => e.name)).toEqual(["only", "sub", "readme.txt"]);
  });
  it("honours descending within the folder/file groups", () => {
    const out = sortEntries(rows, { key: "name", foldersFirst: true, ascending: false });
    expect(out.map((e) => e.name)).toEqual(["sub", "only", "readme.txt"]);
  });
});
