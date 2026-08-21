/// The provenance halo (provenance-halo.md, PH-R4): a pull-only, plain-language
/// answer to "where did this come from, who touched it, why is it here". The halo
/// renders a typed ProvenanceChain, model-free and offline.
///
/// The honesty discipline is load-bearing: the origin is an UNSIGNED,
/// trust-on-assertion DB key, NOT a cryptographic attestation, so the prose must
/// never imply verification ("you authored this" reads as verified when it is an
/// unsigned `user` key). Attested phrasing is reserved for the one origin with real
/// backing - an external file carrying a C2PA content credential. Fidelity markers
/// never overclaim: a `pid` step is "a process", never "app X".
///
/// Mock-vs-live: fixture-backed. The caller-scoped read op (PH-R1, security-critical,
/// shared with the Living Capability Graph's access_grants) + `provenance_of` + the
/// S18-A content-origin persistence + the ebpf pid->app resolution are coder seams.

import { invoke } from "@tauri-apps/api/core";
import { relativeTime, type Translate } from "@arlen/ui-kit/i18n";

/// How Arlen came to record a step (daemons/knowledge provenance.rs).
export type Provenance = "user" | "graph" | "external" | "model" | "agent";

/// How confident the actor resolution is - never rendered as more than it is.
export type Fidelity = "resolved" | "pid" | "proxy";

/// One hop of lineage.
export interface ProvenanceStep {
  /// Which fact this hop states, for the graph/external cases.
  ///
  /// A closed set rather than a phrase, because the sentence around it differs
  /// per language: English puts the verb first ("Last opened by X"), German puts
  /// it last. A free-form verb could only ever be glued to a fixed frame.
  relation?: "partOf" | "lastOpenedBy" | "downloadedFrom";
  actor: string;
  origin: Provenance;
  /// WHEN, as epoch milliseconds rather than as a phrase.
  ///
  /// The host used to send "2 hours ago" ready-made, and this window then
  /// interpolated that English into a translated sentence - so a German reader
  /// got half a sentence in each language. `Intl.RelativeTimeFormat` knows every
  /// language's wording, so the instant travels and the words are written here.
  ///
  /// `0` means the host had no timestamp, and reads as "recently" rather than as
  /// 1970.
  ///
  /// Still a rendered string rather than a timestamp, so it arrives in whatever
  /// language wrote it - the fixtures write English and it shows through beside
  /// German sentences. The fix is a timestamp here and one shared relative-time
  /// formatter over `Intl.RelativeTimeFormat`; there are three hand-rolled copies
  /// of that in the tree already (harness, settings, this), so it wants doing
  /// once in the kit rather than a fourth time here.
  when_ms: number;
  fidelity: Fidelity;
  /// Only true when a C2PA content credential actually backs the external origin.
  attested?: boolean;
}

/// The lineage of a piece of content.
export interface ProvenanceChain {
  subject: string;
  steps: ProvenanceStep[];
  /// Whether the trail is complete, or deeper history is gated (never faked).
  horizon: "complete" | "deeper_gated";
  /// True when this chain is a SAMPLE, not this file's real lineage - set when
  /// the `provenance_of` backend is absent and the fixture stands in. The halo
  /// must say so: the fixtures include an `attested` C2PA step, and rendering
  /// invented lineage unlabelled is exactly the overclaim this module forbids.
  mocked?: boolean;
  /// True when a read this chain was built from did not answer, so steps may be
  /// missing. Distinct from `horizon`: that says deeper history exists and is
  /// gated, this says we do not know what we did not get. Both can be true.
  ///
  /// Without it a chain shortened by a failed read is indistinguishable from a
  /// file with a short history - the empty-on-error defect one hop on, where it
  /// is worse, because a short chain does not look like an error at all.
  incomplete?: boolean;
}

/// The sample chain's own clock. A fixture with real instants would drift with
/// the day it is read on, and a screenshot of it would never match twice.
const SAMPLE_NOW = Date.parse("2026-08-14T09:00:00Z");
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

const FIXTURES: Record<string, ProvenanceChain> = {
  default: {
    subject: "budget-2026.xlsx",
    steps: [
      { origin: "user", actor: "you", when_ms: SAMPLE_NOW - 7 * DAY, fidelity: "resolved" },
      {
        relation: "partOf",
        origin: "graph",
        actor: "project Atlas",
        when_ms: SAMPLE_NOW - 3 * DAY,
        fidelity: "resolved",
      },
      {
        relation: "lastOpenedBy",
        origin: "graph",
        actor: "",
        when_ms: SAMPLE_NOW - 2 * HOUR,
        fidelity: "pid",
      },
    ],
    horizon: "deeper_gated",
  },
  external: {
    subject: "report.pdf",
    steps: [
      {
        relation: "downloadedFrom",
        origin: "external",
        actor: "example.com",
        when_ms: SAMPLE_NOW - DAY,
        fidelity: "resolved",
      },
    ],
    horizon: "deeper_gated",
  },
  attested: {
    subject: "photo.jpg",
    steps: [
      {
        origin: "external",
        actor: "an Acme camera",
        when_ms: Date.parse("2024-06-01T12:00:00Z"),
        fidelity: "resolved",
        attested: true,
      },
    ],
    horizon: "complete",
  },
  model: {
    subject: "This summary",
    steps: [
      {
        origin: "model",
        actor: "the assistant",
        when_ms: SAMPLE_NOW - 10 * 60_000,
        fidelity: "resolved",
      },
    ],
    horizon: "complete",
  },
  agent: {
    subject: "This tag",
    steps: [
      {
        origin: "agent",
        actor: "the idle curator",
        when_ms: SAMPLE_NOW - 6 * HOUR,
        fidelity: "resolved",
      },
    ],
    horizon: "complete",
  },
};

/// Load the provenance of a content reference. Live: `provenance_of`; fixture under
/// vite (keyed by a hint in the ref, else the default file chain).
export async function loadProvenance(ref: string): Promise<ProvenanceChain> {
  try {
    return await invoke<ProvenanceChain>("provenance_of", { ref });
  } catch {
    const key = Object.keys(FIXTURES).find((k) => ref.includes(k)) ?? "default";
    // Flagged, never silent: without the backend this is a sample chain about a
    // different file, and an unlabelled origin claim reads as this file's real
    // (sometimes attested) lineage.
    return { ...FIXTURES[key], mocked: true };
  }
}

/// The actor as we may honestly name it - fidelity never overclaims.
function honestActor(t: Translate, s: ProvenanceStep): string {
  if (s.fidelity === "pid") return t("f.prov.aProcess");
  if (s.fidelity === "proxy") return t("f.prov.focusedWindow");
  return s.actor;
}

/// One step as a single honest sentence. The trust caveat is baked in; nothing
/// reads as verified unless a content credential backs it.
///
/// The translator is a parameter: this is called from a render site, and a
/// function reading the store itself would pin whichever language was current
/// when it first ran.
/// How long ago, in the reader's language.
///
/// `relativeTime` is the kit's wrapper over `Intl.RelativeTimeFormat`, which
/// already knows every language's wording - including the ones with a word for
/// "yesterday" - so no catalogue entry is needed per unit and adding a language
/// does not add six strings to write.
///
/// A ZERO or FUTURE instant is not a date. Zero is the host saying it had no
/// timestamp, and a stamp in the future is a clock disagreement; both read as
/// "recently" rather than as 1970 or as "in three hours".
function whenWords(t: Translate, ms: number, loc: string): string {
  if (!Number.isFinite(ms) || ms <= 0 || ms > Date.now()) return t("f.prov.recently");
  return relativeTime(ms, loc);
}

export function stepLine(t: Translate, s: ProvenanceStep, loc: string): string {
  const actor = honestActor(t, s);
  const when = whenWords(t, s.when_ms, loc);
  switch (s.origin) {
    case "user":
      return t("f.prov.user", { when });
    case "graph":
      return s.relation === "lastOpenedBy"
        ? t("f.prov.lastOpenedBy", { actor, when })
        : t("f.prov.partOf", { actor, when });
    case "external":
      if (s.attested) return t("f.prov.attested", { actor, when });
      return s.relation === "downloadedFrom"
        ? t("f.prov.downloadedFrom", { actor, when })
        : t("f.prov.externalPlain", { when });
    case "model":
      return t("f.prov.model", { when });
    case "agent":
      return t("f.prov.agent", { when });
  }
}

/// The horizon line, or null when the trail is complete. Never a faked full trail.
export function horizonLine(
  t: Translate,
  chain: ProvenanceChain,
): string | null {
  return chain.horizon === "deeper_gated" ? t("f.prov.horizon") : null;
}

/// The caveat for a chain built on a read that did not answer, or null.
///
/// Separate from [`horizonLine`] on purpose: "deeper history is gated" tells a
/// person the trail continues and they may not follow it, which is a different
/// thing from "part of this trail could not be read at all". Showing the gated
/// line for a failed read would say we know where the history stops.
export function incompleteLine(
  t: Translate,
  chain: ProvenanceChain,
): string | null {
  return chain.incomplete ? t("f.prov.incomplete") : null;
}
