// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { formatModified, formatSize } from "./format";

const NOW = 1_700_000_000;
const ago = (seconds: number) => NOW - seconds;

describe("formatModified", () => {
  it("speaks the caller's language, not English", () => {
    // The point of the rewrite: none of these words are written in our source.
    expect(formatModified(ago(10), NOW, "en")).toBe("now");
    expect(formatModified(ago(10), NOW, "de")).toBe("jetzt");
    expect(formatModified(ago(600), NOW, "de")).toBe("vor 10 Minuten");
    expect(formatModified(ago(90_000), NOW, "de")).toBe("gestern");
  });

  it("picks the plural arm by the locale's rules rather than a ternary", () => {
    expect(formatModified(ago(3600), NOW, "en")).toBe("1 hour ago");
    expect(formatModified(ago(7200), NOW, "en")).toBe("2 hours ago");
    // Polish splits at 1 / 2-4 / 5+, which no ternary in a component could reach.
    expect(formatModified(ago(5 * 86400), NOW, "pl")).toBe("5 dni temu");
    expect(formatModified(ago(3 * 86400), NOW, "pl")).toBe("3 dni temu");
  });

  it("orders the date fields the way the locale does", () => {
    // The old code hardcoded `toLocaleString("en")` and `May 12, 2025`.
    const old = ago(400 * 86400);
    expect(formatModified(old, NOW, "en")).toMatch(/\d/);
    expect(formatModified(old, NOW, "de")).toMatch(/\./);
  });

  it("has nothing to say about a missing timestamp", () => {
    expect(formatModified(null, NOW, "en")).toBe("");
    expect(formatSize(null)).toBe("");
  });
});
