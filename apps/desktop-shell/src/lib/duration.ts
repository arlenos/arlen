/// How long, written out: "2h 15min", "45min".
///
/// The unit is part of the message rather than concatenated in code. `h` and
/// `min` are English abbreviations; German writes Std. and Min. with a space
/// before them, and the pattern for a duration is not the same everywhere. So
/// the caller passes numbers and the catalog decides the shape.
///
/// Shared because the battery says it twice - once in the popover and once in
/// the top bar's tooltip - and two copies of a duration format is how they end
/// up disagreeing.

import type { Translate } from "@arlen/ui-kit/i18n";

/// `mins` as a duration, or an empty string when there is nothing to say.
export function durationText(t: Translate, mins: number | null): string {
  if (!mins || mins <= 0) return "";
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return h > 0 ? t("sh.dur.hm", { h, m }) : t("sh.dur.m", { m });
}
