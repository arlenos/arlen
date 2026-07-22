/// Active capsules (context-capsule.md §8): the OUTBOUND sovereignty sibling of the
/// App-access page. App-access is "what can reach me"; this is "what I share" - the
/// signed, frozen, expiring, op-count-limited slices of the knowledge graph the user
/// has handed outward. Day-one same-machine.
///
/// Mock-vs-live: fixture-backed. `list_capsules` (enumerate the revoke-set + grant
/// metadata + remaining op-count + the origin label) is NOT built yet, and
/// `revoke_capsule` needs a Tauri wrapper over the daemon's terminal revoke; both
/// are coder seams. The store falls back to a fixture under vite, like grants.ts.
///
/// Revoke is TERMINAL: the durable revoke-set refuses every future read. It is not
/// restorable and it cannot un-send a copy the recipient already holds - so there is
/// no undo, and the copy never says a share "becomes unreadable".

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// A capsule's lifecycle state as the list presents it.
export type CapsuleState = "active" | "expired" | "exhausted";

/// One active (or spent) share, as rendered.
export interface Capsule {
  id: string;
  /// The durable revocation handle the daemon keys on.
  handle: string;
  /// The named thing it was minted from ("Reading list", "Project Atlas notes").
  label: string;
  /// Who can read it: day-one "this machine", else a short verifying-key label.
  audience: string;
  /// A plain, relation-type-level summary of what the slice carries.
  scope: string;
  /// When the grant expires (mandatory), phrased for a person.
  expiresAt: string;
  /// Reads remaining before the op-count is spent (max_ops - ops_used).
  readsLeft: number;
  state: CapsuleState;
}

// Fixtures: a couple of live shares plus one expired and one op-count-exhausted, so
// the spent states render. Static (no Date/random) so the view is deterministic.
const MOCK_CAPSULES: Capsule[] = [
  {
    id: "c1",
    handle: "h-c1",
    label: "Reading list",
    audience: "this machine",
    scope: "12 notes and their tags",
    expiresAt: "in 6 days",
    readsLeft: 12,
    state: "active",
  },
  {
    id: "c2",
    handle: "h-c2",
    label: "Project Atlas notes",
    audience: "laptop, key a1b2…7f",
    scope: "the Atlas project and its files",
    expiresAt: "tomorrow",
    readsLeft: 3,
    state: "active",
  },
  {
    id: "c3",
    handle: "h-c3",
    label: "Trip 2026",
    audience: "phone, key 9c3d…2e",
    scope: "8 places and their links",
    expiresAt: "expired 2 days ago",
    readsLeft: 0,
    state: "expired",
  },
  {
    id: "c4",
    handle: "h-c4",
    label: "Recipe collection",
    audience: "this machine",
    scope: "40 recipes",
    expiresAt: "in 3 weeks",
    readsLeft: 0,
    state: "exhausted",
  },
];

/// The active capsules the list shows (fixture until `list_capsules` lands).
export const capsules = writable<Capsule[]>([]);
export const capsulesLoaded = writable(false);
/// A transient error surfaced if a revoke did not reach the daemon.
export const capsuleNotice = writable<string | null>(null);

/// Load the active capsules. Live: `list_capsules`; fixture under vite.
/// True while the list is the FIXTURE, not shares you actually made. This answers
/// "what data have I sent out and who can still read it", so an unlabelled sample
/// both invents shares that do not exist and implies real ones are absent.
export const capsulesMocked = writable(false);

export async function loadCapsules(): Promise<void> {
  try {
    capsules.set(await invoke<Capsule[]>("list_capsules"));
    capsulesMocked.set(false);
  } catch {
    capsules.set(MOCK_CAPSULES);
    capsulesMocked.set(true);
  }
  capsulesLoaded.set(true);
}

/// Revoke a share. TERMINAL: optimistically drop the row (no restore), then tell the
/// daemon. Live: `revoke_capsule` writes the durable revoke-set; a transport error
/// under vite counts as applied, like grants.ts.
export async function revokeCapsule(id: string): Promise<void> {
  let handle = "";
  let previous: Capsule[] = [];
  capsules.update((list) => {
    previous = list;
    handle = list.find((x) => x.id === id)?.handle ?? "";
    return list.filter((x) => x.id !== id);
  });
  try {
    await invoke("revoke_capsule", { handle });
  } catch (e) {
    // With a real daemon a refused revoke must NOT read as a stopped share: the
    // row goes back and says why. Silently dropping it would tell the user they
    // had cut off access that is still live. Without the runtime there is no
    // daemon to refuse, so the optimistic drop stands.
    if (tauriAvailable) {
      capsules.set(previous);
      capsuleNotice.set(`Could not revoke that share: ${String(e)}`);
    }
  }
}
