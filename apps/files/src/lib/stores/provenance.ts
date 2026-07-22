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

/// How Arlen came to record a step (daemons/knowledge provenance.rs).
export type Provenance = "user" | "graph" | "external" | "model" | "agent";

/// How confident the actor resolution is - never rendered as more than it is.
export type Fidelity = "resolved" | "pid" | "proxy";

/// One hop of lineage.
export interface ProvenanceStep {
  /// The fact verb, for the graph/external cases ("Part of", "Last opened by").
  relation?: string;
  actor: string;
  origin: Provenance;
  /// A coarse, human "when" ("2 hours ago", "last week").
  when: string;
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
}

const FIXTURES: Record<string, ProvenanceChain> = {
  default: {
    subject: "budget-2026.xlsx",
    steps: [
      { origin: "user", actor: "you", when: "last week", fidelity: "resolved" },
      { relation: "Part of", origin: "graph", actor: "project Atlas", when: "3 days ago", fidelity: "resolved" },
      { relation: "Last opened by", origin: "graph", actor: "", when: "2 hours ago", fidelity: "pid" },
    ],
    horizon: "deeper_gated",
  },
  external: {
    subject: "report.pdf",
    steps: [{ relation: "Downloaded from", origin: "external", actor: "example.com", when: "yesterday", fidelity: "resolved" }],
    horizon: "deeper_gated",
  },
  attested: {
    subject: "photo.jpg",
    steps: [{ origin: "external", actor: "an Acme camera", when: "in 2024", fidelity: "resolved", attested: true }],
    horizon: "complete",
  },
  model: {
    subject: "This summary",
    steps: [{ origin: "model", actor: "the assistant", when: "10 minutes ago", fidelity: "resolved" }],
    horizon: "complete",
  },
  agent: {
    subject: "This tag",
    steps: [{ origin: "agent", actor: "the idle curator", when: "overnight", fidelity: "resolved" }],
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
function honestActor(s: ProvenanceStep): string {
  if (s.fidelity === "pid") return "a process";
  if (s.fidelity === "proxy") return "the focused window";
  return s.actor;
}

/// One step as a single honest sentence. The trust caveat is baked in; nothing
/// reads as verified unless a content credential backs it.
export function stepLine(s: ProvenanceStep): string {
  const actor = honestActor(s);
  switch (s.origin) {
    case "user":
      return `Arlen recorded this as yours, ${s.when}.`;
    case "graph":
      return `${s.relation} ${actor}, ${s.when}. Recorded from what Arlen observed.`;
    case "external":
      return s.attested
        ? `Verified as from ${actor}, ${s.when}, by a content credential.`
        : `${s.relation ? `${s.relation} ${actor}` : "Came from an external document"}, ${s.when}. Arlen did not verify this.`;
    case "model":
      return `The assistant asserted this from its reasoning, ${s.when}.`;
    case "agent":
      return `The idle curator consolidated this, ${s.when}.`;
  }
}

/// The horizon line, or null when the trail is complete. Never a faked full trail.
export function horizonLine(chain: ProvenanceChain): string | null {
  return chain.horizon === "deeper_gated" ? "Deeper history isn't available yet." : null;
}
