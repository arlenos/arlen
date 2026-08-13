// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The shape a read comes back in, so no surface has to invent its own way of
// saying "nothing, but for which reason".
//
// A list alone collapses three different facts into one screen: the subsystem is
// not on this machine, the subsystem refused this app, and there genuinely is
// nothing. The first two then read to a person as the third - "you have no
// accounts", "this file belongs to no project" - which is a missing or refused
// subsystem wearing the costume of an answer.
//
// The Rust side is `ReadOutcome<T>` in the files backend and `NetworkPlaces` in
// the remote-places command; both serialize with a `state` tag, so a caller must
// branch and cannot destructure an absence into an empty list by accident.

/** A read that produced rows, or a reason it produced none. */
export type ReadOutcome<T> =
  | { state: "unavailable"; reason: string }
  | { state: "denied"; reason: string }
  | { state: "rows"; rows: T[] };

/** The rows a read produced; empty for every non-`rows` state. */
export function rows<T>(r: ReadOutcome<T> | null | undefined): T[] {
  return r && r.state === "rows" ? r.rows : [];
}

/**
 * Which message key describes this outcome, or null when there is nothing to
 * explain because the read produced rows.
 *
 * The keys are shared (`f.read.*`) on purpose: one pattern applied everywhere
 * beats six honest answers phrased six ways, and a person who learns what
 * "Not available on this system" means in one panel should not have to learn it
 * again in the next.
 */
export function reasonKey<T>(
  r: ReadOutcome<T> | null | undefined,
): string | null {
  if (!r) return null;
  if (r.state === "unavailable") return "f.read.unavailable";
  if (r.state === "denied") return "f.read.denied";
  return r.rows.length === 0 ? "f.read.empty" : null;
}
