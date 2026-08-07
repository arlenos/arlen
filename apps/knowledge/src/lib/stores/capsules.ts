/// The capsule model (context-capsule.md, KA-R6): the active capsules with a
/// one-gesture revoke, and the MINT flow - share a named slice of the graph
/// with an audience, an expiry and a read budget, behind the MANDATORY
/// relation-type-level over-share preview (decision 11: "follows MENTIONS,
/// reaches 1,240 nodes", deselectable; a raw node ceiling is the wrong UX).
/// Minting is a human act (decision 14): only this surface mints, never an
/// agent path. Day-one reach is same-machine (decision 1), so the audience
/// list offers this machine's readers; external egress stays behind its
/// human-gated flag and is not offered here.
///
/// Mock-vs-live: `knowledge_capsules` / `knowledge_capsule_mint` /
/// `knowledge_capsule_revoke` and the preview read are coder seams over the
/// built capsule.rs; under vite fixtures stand in and `capsulesMocked` says so.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One active (or spent) capsule, display-shaped.
export interface Capsule {
  id: string;
  /// What was shared, in the user's words ("Thesis slice").
  label: string;
  /// Who can read it.
  audience: string;
  /// Plain-words expiry ("in 5 days").
  expiresAt: string;
  /// Remaining read budget.
  readsLeft: number;
  state: "active" | "expired" | "spent";
}

/// True while the list is the FIXTURE, not the broker.
export const capsulesMocked = writable(false);
/// The capsules, or null before the read settles.
export const capsules = writable<Capsule[] | null>(null);

/// A shareable named thing: a project or a saved search - never a query.
export interface Shareable {
  kind: "project" | "search";
  name: string;
}

/// One relation type the selected slice would follow, with its reach - the
/// over-share preview's row. `included` is the user's call; a risky row
/// (high-degree reach) starts EXCLUDED so the dangerous walk is opt-in.
export interface LinkPreview {
  relation: string;
  nodes: number;
  risky: boolean;
  included: boolean;
}

// i18n-foreign: a capsule is named by whoever minted it, after their own work.
const FIXTURE_CAPSULES: Capsule[] = [
  { id: "c-1", label: "Thesis slice", audience: "The assistant", expiresAt: "in 5 days", readsLeft: 37, state: "active" },
];

/// The named things the mint scope picker offers (the story's projects + the
/// saved searches; live this is a typed read).
export const SHAREABLES: Shareable[] = [
  { kind: "project", name: "Thesis" },
  { kind: "project", name: "Arlen OS" },
  { kind: "project", name: "Website redesign" },
  { kind: "search", name: "Papers I have not read" },
];

// Per shareable, what its one-hop walk would follow. MENTIONS is the classic
// high-degree trap (a person or tag linked to everything) - risky, excluded
// by default.
const PREVIEWS: Record<string, LinkPreview[]> = {
  "Thesis": [
    { relation: "FILE_PART_OF", nodes: 214, risky: false, included: true },
    { relation: "AUTHORED_BY", nodes: 38, risky: false, included: true },
    { relation: "MENTIONS", nodes: 1240, risky: true, included: false },
  ],
  "Arlen OS": [
    { relation: "FILE_PART_OF", nodes: 486, risky: false, included: true },
    { relation: "TOUCHED_BY", nodes: 122, risky: false, included: true },
    { relation: "MENTIONS", nodes: 2050, risky: true, included: false },
  ],
  "Website redesign": [
    { relation: "FILE_PART_OF", nodes: 32, risky: false, included: true },
    { relation: "IN_SESSION", nodes: 9, risky: false, included: true },
  ],
  "Papers I have not read": [
    { relation: "IMPORTED_FROM", nodes: 12, risky: false, included: true },
    { relation: "CITES", nodes: 640, risky: true, included: false },
  ],
};

/// The preview rows for a shareable. Live: the mint-path preview read (seam);
/// the fixture stands in per name.
export async function previewFor(name: string): Promise<LinkPreview[]> {
  try {
    return await invoke<LinkPreview[]>("knowledge_capsule_preview", { name });
  } catch {
    return (PREVIEWS[name] ?? []).map((p) => ({ ...p }));
  }
}

/// Load the capsule list. Live: `knowledge_capsules` (seam).
export async function loadCapsules(): Promise<void> {
  try {
    const live = await invoke<Capsule[]>("knowledge_capsules", {});
    capsules.set(live);
    capsulesMocked.set(false);
  } catch {
    capsules.set(FIXTURE_CAPSULES.map((c) => ({ ...c })));
    capsulesMocked.set(true);
  }
}

/// Revoke: stops every future read; it cannot pull back a copy already made
/// (decision 4 - never phrased as un-send). Live: `knowledge_capsule_revoke`.
export async function revokeCapsule(id: string): Promise<void> {
  capsules.update((l) => (l ? l.filter((c) => c.id !== id) : l));
  try {
    await invoke("knowledge_capsule_revoke", { id });
  } catch {
    // No broker under vite: the optimistic removal stands.
  }
}

/// Mint (a human act, this surface only). Live: `knowledge_capsule_mint`;
/// the optimistic entry stands under vite behind the mocked banner.
export async function mintCapsule(
  name: string,
  audience: string,
  expiryDays: number,
  reads: number,
  links: LinkPreview[]
): Promise<void> {
  const entry: Capsule = {
    id: `c-${Math.random().toString(36).slice(2, 8)}`,
    label: `${name} slice`,
    audience,
    expiresAt: expiryDays === 1 ? "in 1 day" : `in ${expiryDays} days`,
    readsLeft: reads,
    state: "active",
  };
  capsules.update((l) => (l ? [entry, ...l] : [entry]));
  try {
    await invoke("knowledge_capsule_mint", {
      name,
      audience,
      expiryDays,
      reads,
      relations: links.filter((l) => l.included).map((l) => l.relation),
    });
  } catch {
    // No broker under vite: the optimistic entry stands.
  }
}
