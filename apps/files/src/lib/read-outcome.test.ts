// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Three facts that look identical on screen - the subsystem is not here, it
// refused this app, and there is genuinely nothing - and one function that keeps
// them apart. Every "you have no accounts" this app has ever wrongly said came
// from collapsing them, so the branch is pinned rather than trusted.
import { describe, it, expect } from "vitest";
import { rows, reasonState, type ReadOutcome } from "./read-outcome.js";

const missing: ReadOutcome<string> = { state: "unavailable", reason: "no daemon" };
const refused: ReadOutcome<string> = { state: "denied", reason: "not granted" };
const empty: ReadOutcome<string> = { state: "rows", rows: [] };
const some: ReadOutcome<string> = { state: "rows", rows: ["a", "b"] };

describe("rows", () => {
  it("hands back the rows of a read that produced some", () => {
    expect(rows(some)).toEqual(["a", "b"]);
  });

  it("is empty for every other state, including no read at all", () => {
    expect(rows(empty)).toEqual([]);
    expect(rows(missing)).toEqual([]);
    expect(rows(refused)).toEqual([]);
    expect(rows(null)).toEqual([]);
    expect(rows(undefined)).toEqual([]);
  });
});

describe("reasonState", () => {
  it("names each of the three absences", () => {
    expect(reasonState(missing)).toBe("unavailable");
    expect(reasonState(refused)).toBe("denied");
    expect(reasonState(empty)).toBe("empty");
  });

  it("has no reason to give when the read produced rows", () => {
    expect(reasonState(some)).toBe(null);
  });

  it("says nothing about a read that has not happened yet", () => {
    // Not "empty": before the first read there is no claim to make, and a
    // surface that renders "nothing here" during its own load is telling the
    // person something it does not know.
    expect(reasonState(null)).toBe(null);
    expect(reasonState(undefined)).toBe(null);
  });
});
