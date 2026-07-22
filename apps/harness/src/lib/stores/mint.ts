/// The capsule mint flow (context-capsule.md §8): the deliberate, human-only
/// "share a slice of my context" act. The user picks a named thing from the
/// knowledge graph, sets a recipient + a mandatory expiry + an op-count, reviews
/// the mandatory relation-type over-share preview, and mints a signed capsule that
/// then appears (revocable) in Settings > Privacy > Shared context.
///
/// Mock-vs-live: fixture-backed. `capsule_scope_options` (the named things to
/// share), `capsule_preview` (relation-type reach counts + a sensitive-field flag
/// for a scope), and `capsule_mint` (knowledge 0x07 materialize -> capsuled
/// sign/store) are coder seams; the store falls back to fixtures under vite.
///
/// Mint is a HUMAN act - this surface is never wired to any agent path (the agent
/// may propose a share in suggest-mode, it never mints one).

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// A named thing the user can share (a project, a saved view, a selection).
export interface ScopeOption {
  id: string;
  label: string;
  description: string;
}

/// One relation type the slice would follow, and how far it reaches.
export interface PreviewRelation {
  type: string;
  /// A plain description of what the relation pulls in.
  label: string;
  reach: number;
}

/// The over-share preview for a scope: the core count, the relations it follows,
/// and whether it holds sensitive fields (excluded by default).
export interface Preview {
  baseCount: number;
  relations: PreviewRelation[];
  hasSensitive: boolean;
}

/// The mint form the steps fill in.
export interface MintForm {
  scopeId: string | null;
  audience: string;
  expiry: string;
  opCount: string;
  /// Relation types the user dropped from the share (over-share preview).
  dropped: string[];
  includeSensitive: boolean;
}

const MOCK_SCOPE_OPTIONS: ScopeOption[] = [
  { id: "reading-list", label: "Reading list", description: "12 notes you saved to read" },
  { id: "project-atlas", label: "Project Atlas", description: "the project and its files" },
  { id: "trip-2026", label: "Trip 2026", description: "8 places and their links" },
  { id: "selection", label: "Current selection", description: "3 items you have open" },
];

const MOCK_PREVIEW: Preview = {
  baseCount: 12,
  hasSensitive: true,
  relations: [
    { type: "TAGGED", label: "their tags", reach: 45 },
    { type: "MENTIONS", label: "notes they mention", reach: 1240 },
    { type: "AUTHORED_BY", label: "the people who wrote them", reach: 3 },
  ],
};

const EMPTY_FORM: MintForm = {
  scopeId: null,
  audience: "this-machine",
  expiry: "1w",
  opCount: "20",
  dropped: [],
  includeSensitive: false,
};

/// Open state + the current step (0 Scope, 1 Recipient & limits, 2 Review).
export const mintOpen = writable(false);
export const mintStep = writable(0);
export const mintForm = writable<MintForm>({ ...EMPTY_FORM });
/// Loaded via IPC, so writable (the Svelte-5 IPC re-render caveat).
export const scopeOptions = writable<ScopeOption[]>([]);
export const preview = writable<Preview | null>(null);
/// The label of the just-minted capsule, or null before mint (drives the result).
export const mintResult = writable<string | null>(null);

/// Open the flow fresh: reset the form, load the named things.
export function openMint(): void {
  mintForm.set({ ...EMPTY_FORM, dropped: [] });
  mintStep.set(0);
  mintResult.set(null);
  preview.set(null);
  mintOpen.set(true);
  void loadScopeOptions();
}

export function closeMint(): void {
  mintOpen.set(false);
}

/// The named things the user can share. Live: `capsule_scope_options`.
/// True while the scope menu and preview are FIXTURES, not your real data. The
/// options read as things you actually saved ("Reading list - 12 notes you saved
/// to read"), so unlabelled the user is choosing what to share out of an invented
/// menu, against invented reach numbers.
export const mintMocked = writable(false);

/// Set when a real mint was refused. Empty otherwise.
export const mintError = writable("");

export async function loadScopeOptions(): Promise<void> {
  try {
    scopeOptions.set(await invoke<ScopeOption[]>("capsule_scope_options"));
    mintMocked.set(false);
  } catch {
    scopeOptions.set(MOCK_SCOPE_OPTIONS);
    mintMocked.set(true);
  }
}

/// The over-share preview for the picked scope. Live: `capsule_preview`.
export async function loadPreview(scopeId: string): Promise<void> {
  preview.set(null);
  try {
    preview.set(await invoke<Preview>("capsule_preview", { scopeId }));
  } catch {
    preview.set(MOCK_PREVIEW);
  }
}

/// Mint the capsule. Live: `capsule_mint` (materialize + sign + store). Records the
/// scope label for the result screen either way.
export async function mint(form: MintForm, scopeLabel: string): Promise<void> {
  mintError.set("");
  try {
    await invoke("capsule_mint", {
      scopeId: form.scopeId,
      audience: form.audience,
      expiry: form.expiry,
      opCount: form.opCount,
      dropped: form.dropped,
      includeSensitive: form.includeSensitive,
    });
  } catch (e) {
    // A REAL refusal must not reach the success screen. `mintResult` used to be
    // set unconditionally, so a mint the daemon rejected (bad audience, expiry,
    // no signing key) still confirmed - telling the user a signed capsule of
    // their data exists and was shared when none was made. Without the runtime
    // there is no daemon to refuse, so the flow still confirms against the
    // fixture and stays reviewable.
    if (tauriAvailable) {
      mintError.set(`Could not create that capsule: ${String(e)}`);
      return;
    }
  }
  mintResult.set(scopeLabel);
}
