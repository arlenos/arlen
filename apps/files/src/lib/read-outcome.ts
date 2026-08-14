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

/** Why a read showed nothing: absent, refused, or genuinely empty. */
export type EmptyReason = "unavailable" | "denied" | "empty";

/**
 * Which of the three this outcome is, or null when it produced rows.
 *
 * The STATE is shared and the sentence is not. Three fixed terms - absent,
 * refused, empty - are what keeps the app coherent, because they are three
 * different next steps for a person: the feature is not on this machine, they
 * were not allowed, or there is nothing there. One shared SENTENCE would have to
 * be vague enough to fit every surface, and vague is how "your places could not
 * be read" turns into "not available on this system" about places that are.
 *
 * So each surface names the thing the person was looking at, in its own words,
 * and this decides which of the three it is saying.
 *
 * WHERE THIS DOES NOT APPLY, which is worth writing down because the same app
 * does the opposite one file over and both are right. The Places sidebar drops a
 * whole GROUP when its read fails - "no section beats an empty one" - rather than
 * showing a Bookmarks heading with a reason under it.
 *
 * The line is what the person asked for. A listing is the answer to a navigation
 * they just performed, so silence there leaves them to conclude the folder is
 * empty; the reason has to be said. A sidebar group is chrome they did not ask
 * for, and a heading over an explanation is a worse answer than no heading -
 * it spends the space arguing about something nobody was looking at.
 *
 * So: name the reason on what was requested, omit the section that was not. Do
 * not "fix" one of these to match the other.
 */
export function reasonState<T>(
  r: ReadOutcome<T> | null | undefined,
): EmptyReason | null {
  if (!r) return null;
  if (r.state === "unavailable") return "unavailable";
  if (r.state === "denied") return "denied";
  return r.rows.length === 0 ? "empty" : null;
}
