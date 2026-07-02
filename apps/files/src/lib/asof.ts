/// The "as of" time-travel presets, shared by the info panel's per-file lineage
/// read and the header's whole-listing toggle. Only project membership is
/// bitemporal in the graph, so a past cutoff is the meaningful slice; other
/// locations read live regardless.

import { writable } from "svelte/store";

/// The preset choices, relative to now. "now" is the live read.
export const AS_OF_OPTIONS = [
  { value: "now", label: "Now" },
  { value: "1d", label: "1 day ago" },
  { value: "1w", label: "1 week ago" },
  { value: "1m", label: "1 month ago" },
  { value: "3m", label: "3 months ago" },
];

const DAY_MICROS = 86_400_000_000;
const AS_OF_DELTAS: Record<string, number> = {
  "1d": DAY_MICROS,
  "1w": 7 * DAY_MICROS,
  "1m": 30 * DAY_MICROS,
  "3m": 90 * DAY_MICROS,
};

/// Epoch-micros for a preset, or null for "now" (a live read).
export function choiceToMicros(v: string): number | null {
  return v === "now" ? null : Date.now() * 1000 - (AS_OF_DELTAS[v] ?? 0);
}

/// The listing-level time-travel: a global mode that lists the current
/// (project) location as of this preset. "now" is live. The adapter reads it;
/// the header sets it.
export const viewAsOfChoice = writable("now");
