// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: MIT
//
// The chooser answers in URIs and the hosts take paths. The step between them
// is where a name with a space in it gets lost.
import { describe, it, expect } from "vitest";
import { pathOfUri } from "./pick-file.js";

describe("pathOfUri", () => {
  it("reads a plain local file", () => {
    expect(pathOfUri("file:///home/ada/letter.eml")).toBe("/home/ada/letter.eml");
  });

  it("decodes what the portal encoded", () => {
    expect(pathOfUri("file:///home/ada/M%C3%A4rz%20notes.pdf")).toBe("/home/ada/März notes.pdf");
  });

  it("refuses anything that is not a local file", () => {
    expect(pathOfUri("https://example.org/a.pdf")).toBeNull();
    expect(pathOfUri("not a uri")).toBeNull();
  });
});
