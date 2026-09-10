// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The source column names a program the way a person knows it, in both
// spellings the graph records, and shows an unknown id as it is rather than
// inventing a name for it.
import { describe, it, expect } from "vitest";
import { sourceName } from "./sources.js";

describe("sourceName", () => {
  it("names the first-party applications in both spellings", () => {
    expect(sourceName("files")).toBe("Files");
    expect(sourceName("dev.arlen.files")).toBe("Files");
    expect(sourceName("org.arlen.text-editor")).toBe("Text editor");
    expect(sourceName("foot")).toBe("Terminal");
  });

  it("shows an unknown id as it is", () => {
    expect(sourceName("md.obsidian")).toBe("md.obsidian");
    expect(sourceName("Zotero bridge")).toBe("Zotero bridge");
  });
});
