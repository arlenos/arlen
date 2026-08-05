/// The "as of" time-travel presets, shared by the info panel's per-file lineage
/// read and the header's whole-listing toggle. Only project membership is
/// bitemporal in the graph, so a past cutoff is the meaningful slice; other
/// locations read live regardless.

import { derived, writable } from "svelte/store";

import { t } from "$lib/i18n/messages";

/// The preset choices, relative to now. "now" is the live read.
///
/// A derived store rather than a plain array: the labels are shown to the user,
/// and a module-level constant would hold whatever the translator said at import
/// and never follow a locale switch. Deriving keeps the generic select that
/// renders these ignorant of i18n - it still receives plain `{value, label}`.
export const asOfOptions = derived(t, ($t) => [
  { value: "now", label: $t("f.asof.now") },
  { value: "1d", label: $t("f.asof.1d") },
  { value: "1w", label: $t("f.asof.1w") },
  { value: "1m", label: $t("f.asof.1m") },
  { value: "3m", label: $t("f.asof.3m") },
]);

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
