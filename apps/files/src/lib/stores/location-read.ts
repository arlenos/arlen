// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Why the current virtual location listed nothing.
//
// The kit's `BrowserAdapter.list` hands back `FileEntry[]`, and that contract is
// not ours to change, so the reason travels beside the rows rather than inside
// them. The adapter writes here on every virtual listing; the status bar reads it.
//
// Without this a project whose members the graph could not be asked for renders as
// "0 items", which is a count nobody measured - the same shape as the invented
// printers and the alarm nobody set.

import { writable } from "svelte/store";
import {
  reasonState,
  type EmptyReason,
  type ReadOutcome,
} from "$lib/read-outcome";

/// Why the last virtual listing showed nothing, or null when it produced rows.
/// The status bar turns this into a sentence about the LOCATION, which is what
/// the person clicked; the state is what it shares with the rest of the app.
export const locationReadReason = writable<Exclude<
  EmptyReason,
  "empty"
> | null>(null);

/// Record a listing's outcome and hand back its rows for the adapter to sort.
///
/// `empty` is deliberately NOT carried: an empty location already reads correctly
/// as "0 items" in the status bar, and saying "Nothing here" underneath a count of
/// zero is the same fact twice.
export function recordListing<T>(outcome: ReadOutcome<T>): T[] {
  const state = reasonState(outcome);
  locationReadReason.set(state === "empty" || state === null ? null : state);
  return outcome.state === "rows" ? outcome.rows : [];
}

/// A real-filesystem listing: no graph was consulted, so no reason applies.
export function clearListingReason(): void {
  locationReadReason.set(null);
}
