/// The words the extension surfaces share: a capability label as a chip and
/// as a sentence, a health as a readout value, an origin line, and a time in
/// the reader's language. Out of the pages so the list and the detail cannot
/// drift apart, and so a rule can be read without rendering a component.
import type { Translate } from "@arlen/ui-kit/i18n";
import type { Extension, Health } from "$lib/stores/extensions";

const KNOWN = new Set(["network", "filesystem", "notifications", "clipboard", "audio", "system"]);

/// A capability label as the short chip on a list row: "Network", "Files",
/// "Reads system.File". The store's coarse words, not a sentence.
export function capChip(cap: string, t: Translate): string {
  if (KNOWN.has(cap)) return t(`s.ext.chip.${cap}`);
  if (cap.startsWith("read:")) return t("s.ext.chip.graphRead", { what: cap.slice(5) });
  if (cap.startsWith("write:")) return t("s.ext.chip.graphWrite", { what: cap.slice(6) });
  return cap.replace(/[._-]/g, " ");
}

/// A capability label as the sentence the store speaks ("Talks to the
/// network"), for the detail page. An identifier the catalogue does not know is
/// shown de-mangled rather than hidden: an unknown grant is still a grant.
export function capSentence(cap: string, t: Translate): string {
  if (KNOWN.has(cap)) return t(`s.ext.cap.${cap}`);
  if (cap.startsWith("read:")) return t("s.ext.cap.graphRead", { what: cap.slice(5) });
  if (cap.startsWith("write:")) return t("s.ext.cap.graphWrite", { what: cap.slice(6) });
  return cap.replace(/[._-]/g, " ");
}

/// The health as a readout value with its posture for the dot family: a value,
/// never a sentence (the row beside it says "Running").
export function healthReadout(
  health: Health,
  t: Translate,
): { posture: "ours" | "off" | "away" | "unknown"; text: string } {
  if (health === "active") return { posture: "ours", text: t("s.ext.health.active") };
  if (health === "disabled") return { posture: "off", text: t("s.ext.health.disabled") };
  if (health === "unknown") return { posture: "unknown", text: t("s.ext.health.unknown") };
  return { posture: "away", text: t("s.ext.health.failed") };
}

/// The reason a failed extension gave, or null.
export function failureReason(health: Health): string | null {
  return typeof health === "object" ? health.failed : null;
}

/// Where it came from, in one line: the kind, and the source when known.
export function originLine(e: Extension, t: Translate): string {
  const kind = t(`s.ext.kindOne.${e.kind}`);
  return e.provenance ? t("s.ext.originFrom", { kind, source: e.provenance }) : kind;
}

/// A past moment as the reader writes it: "2 days ago", "just now".
export function ago(micros: number, loc: string, t: Translate): string {
  const seconds = Math.max(0, Math.round((Date.now() * 1000 - micros) / 1_000_000));
  if (seconds < 60) return t("s.ext.justNow");
  const rtf = new Intl.RelativeTimeFormat(loc, { numeric: "auto" });
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return rtf.format(-minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (hours < 24) return rtf.format(-hours, "hour");
  const days = Math.round(hours / 24);
  if (days < 30) return rtf.format(-days, "day");
  return rtf.format(-Math.round(days / 30), "month");
}
