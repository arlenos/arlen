/// Everything that extends this system, as one list (shell-extension-model.md:
/// "one management surface"). Apps, shell modules and bridges are three
/// mechanisms, but from the user's side they are one question - what is
/// installed, what may it reach, is it running, how do I take it back - and
/// `extensions_list` answers it in one inventory with one capability
/// vocabulary. This store is the surface's reading of that wire, plus the two
/// per-extension calls: what the machine has seen it do, and the revoke.
///
/// Under vite the fixture stands in and the surface says so. It carries every
/// state the pages draw - the three kinds, all four healths, observed and
/// unobserved and unmeasured capabilities - because those are the states that
/// are only ever looked at here.
import { derived, get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// Which mechanism an extension is. Lowercase on the wire.
export type ExtensionKind = "app" | "module" | "bridge";

/// What an extension is doing now. `failed` carries what it said.
export type Health = "active" | "disabled" | "unknown" | { failed: string };

/// One row of the inventory, exactly as `extensions_list` serialises it.
export interface Extension {
  id: string;
  name: string;
  kind: ExtensionKind;
  /// Coarse labels in the shared vocabulary: `network`, `filesystem`,
  /// `notifications`, `clipboard`, `system`, and verbatim `read:<scope>` /
  /// `write:<scope>` graph scopes. Empty means it asked for nothing.
  capabilities: string[];
  provenance: string | null;
  health: Health;
}

/// What the audit ledger says about one declared capability.
export type Observation =
  | { state: "observed"; actions: number; lastMicros: number }
  | { state: "notObserved" }
  | {
      state: "notMeasured";
      reason: "noFeed" | "ledgerUnavailable" | "actorUnknown" | "notAttributed";
    };

/// One declared capability with what was seen of it.
export interface DeclaredCapability {
  label: string;
  observation: Observation;
}

/// What a revoke did, per step. A partial result is normal.
export interface RevokeReport {
  revoked: string[];
  failed: string[];
  residue: string[];
}

/// Where the inventory stands: being read, read from this machine, not
/// readable, or the sample under vite.
export type InventoryState = "loading" | "live" | "unreadable" | "sample";

export const extensions = writable<Extension[]>([]);
export const inventoryState = writable<InventoryState>("loading");
export const inventoryMocked = derived(inventoryState, ($s) => $s === "sample");

const KINDS: ExtensionKind[] = ["app", "module", "bridge"];

/// A kind off the URL, or null for a word that is not one.
export function kindOf(word: string): ExtensionKind | null {
  return (KINDS as string[]).includes(word) ? (word as ExtensionKind) : null;
}

/// The reach families a person can filter by, in the order the filter lists
/// them. `graph` matches both read and write scopes.
export type Reach = "any" | "network" | "filesystem" | "graph" | "clipboard" | "notifications" | "system";

/// Whether a capability label falls under a reach family.
export function reaches(label: string, reach: Reach): boolean {
  if (reach === "any") return true;
  if (reach === "graph") return label.startsWith("read:") || label.startsWith("write:");
  return label === reach;
}

// ---------------------------------------------------------------------------
// Fixture: the states the pages draw, and nothing that reads as this machine.
// ---------------------------------------------------------------------------

const FIXTURE: Extension[] = [
  { id: "dev.arlen.notes", name: "Notes", kind: "app", capabilities: ["filesystem", "read:system.File", "write:shared.Note"], provenance: "arlen-core", health: "active" },
  { id: "org.example.mapper", name: "Mapper", kind: "app", capabilities: ["network", "filesystem"], provenance: "store", health: "active" },
  { id: "org.example.quiet", name: "Quiet", kind: "app", capabilities: [], provenance: "local", health: "unknown" },
  { id: "wp.calc", name: "Calculator", kind: "module", capabilities: ["clipboard"], provenance: "forage", health: "active" },
  { id: "wp.weather", name: "Weather", kind: "module", capabilities: ["network", "notifications"], provenance: "forage", health: "disabled" },
  { id: "wp.tally", name: "Tally", kind: "module", capabilities: ["read:shared.Person", "system"], provenance: "user", health: { failed: "the host refused a capability the manifest did not declare" } },
  { id: "md.obsidian", name: "Obsidian notes", kind: "bridge", capabilities: ["write:md.obsidian", "filesystem"], provenance: "cookbook", health: "active" },
  { id: "cal.ics", name: "Calendar files", kind: "bridge", capabilities: ["write:cal.ics"], provenance: "cookbook", health: "unknown" },
];

const NOW_MICROS = Date.now() * 1000;
const DAY_MICROS = 86_400_000_000;

function fixtureObserved(ext: Extension): DeclaredCapability[] {
  if (ext.kind === "module") {
    return ext.capabilities.map((label) => ({ label, observation: { state: "notMeasured", reason: "actorUnknown" } }));
  }
  return ext.capabilities.map((label, i) => {
    if (i === 0) return { label, observation: { state: "observed", actions: 12, lastMicros: NOW_MICROS - 2 * DAY_MICROS } };
    if (i === 1) return { label, observation: { state: "notObserved" } };
    return { label, observation: { state: "notMeasured", reason: "noFeed" } };
  });
}

function fixtureRevoke(ext: Extension): RevokeReport {
  if (ext.kind === "bridge") {
    return {
      revoked: [`${ext.id} can no longer write`],
      failed: [],
      residue: [
        `${ext.name} can write nothing further.`,
        `Everything ${ext.id} already ingested stays in your knowledge graph. Removing it is a separate step that does not exist yet.`,
      ],
    };
  }
  return {
    revoked: ext.capabilities.slice(0, 1).map((c) => `giving up ${c}`),
    failed: ext.capabilities.length > 1 ? [`${ext.capabilities[1]} is essential to this app and cannot be removed`] : [],
    residue: [`${ext.name} keeps anything it already read or wrote; this only stops it doing more.`],
  };
}

// ---------------------------------------------------------------------------
// Reads and the one write.
// ---------------------------------------------------------------------------

/// Read the inventory. Live: `extensions_list`; under vite the fixture; on a
/// host that does not answer, `unreadable` and an empty list, never a sample.
export async function loadExtensions(): Promise<void> {
  if (get(inventoryState) !== "live") inventoryState.set("loading");
  try {
    extensions.set(await invoke<Extension[]>("extensions_list"));
    inventoryState.set("live");
  } catch {
    if (tauriAvailable) {
      extensions.set([]);
      inventoryState.set("unreadable");
    } else {
      extensions.set(structuredClone(FIXTURE));
      inventoryState.set("sample");
    }
  }
}

/// One extension by kind and id, from what is loaded.
export function findExtension(kind: ExtensionKind, id: string): Extension | undefined {
  return get(extensions).find((e) => e.kind === kind && e.id === id);
}

/// What this machine has seen the extension do, per declared capability.
/// `null` when the question could not be asked at all.
export async function loadObserved(ext: Extension): Promise<DeclaredCapability[] | null> {
  try {
    return await invoke<DeclaredCapability[]>("extensions_observed", { id: ext.id, kind: ext.kind });
  } catch {
    return tauriAvailable ? null : fixtureObserved(ext);
  }
}

/// Take back everything the extension holds. The report says what went, what
/// did not and why, and what this does not undo. `null` when the host refused
/// to run it at all.
export async function revokeExtension(ext: Extension): Promise<RevokeReport | null> {
  try {
    const report = await invoke<RevokeReport>("extensions_revoke", { id: ext.id, kind: ext.kind });
    await loadExtensions();
    return report;
  } catch {
    if (tauriAvailable) return null;
    const report = fixtureRevoke(ext);
    extensions.update((all) =>
      all.map((e) => (e.kind === ext.kind && e.id === ext.id ? { ...e, capabilities: e.capabilities.slice(1) } : e)),
    );
    return report;
  }
}
