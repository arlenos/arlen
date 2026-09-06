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

/// How long ago, compactly: "now", "4m", "2h" - and their German equivalents.
///
/// It lives here rather than in the undo popover that wanted it first, because
/// this file is already where the shell knows how to say a length of time and a
/// second copy is how the three `readsAsInternal` copies happened. The compact
/// form is deliberate: an undo row is one line, so `relativeTime`'s "vor 4
/// Minuten" would not fit even though it is the more natural sentence.
export function agoText(t: Translate, seconds: number): string {
  const s = Math.max(0, seconds);
  if (s < 90) return t("sh.dur.now");
  const m = Math.floor(s / 60);
  if (m < 60) return t("sh.dur.agoM", { n: m });
  return t("sh.dur.agoH", { n: Math.floor(m / 60) });
}
