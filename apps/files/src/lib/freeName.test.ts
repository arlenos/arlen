import { describe, it, expect } from "vitest";
import { freeName } from "./freeName";

describe("freeName", () => {
  it("uses the plain name when nothing has it", () => {
    expect(freeName("New folder", ["a.txt"])).toBe("New folder");
  });

  it("counts past the ones that are taken", () => {
    expect(freeName("New folder", ["New folder"])).toBe("New folder 2");
    expect(freeName("New folder", ["New folder", "New folder 2"])).toBe("New folder 3");
  });

  it("fills a gap rather than always taking the highest", () => {
    expect(freeName("New folder", ["New folder", "New folder 3"])).toBe("New folder 2");
  });

  /// A Linux directory is case-sensitive, and refusing `Notes` because `notes`
  /// exists would refuse a name the filesystem would have taken.
  it("compares exactly, because the filesystem does", () => {
    expect(freeName("Notes", ["notes"])).toBe("Notes");
  });

  it("gives the backend the plain name back rather than counting for ever", () => {
    const many = ["New folder", ...Array.from({ length: 1000 }, (_, i) => `New folder ${i + 2}`)];
    expect(freeName("New folder", many)).toBe("New folder");
  });
});
