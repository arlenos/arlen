/// The unified consent dialog (system-dialog-plan.md): the one REQUEST-moment
/// surface every permission prompt routes into. The broker resolves a severity
/// tier and hands the shell a PendingView; the shell renders the right dialog and
/// returns an outcome. This is the sibling of the App-access review/revoke page.
///
/// Mock-vs-live: fixture-backed. The `consent_fetch` / `consent_resolve` Tauri
/// commands wrapping the broker's `ControlClient`, and the broker-signal listener
/// that drives the fetch, are coder seams; under vite the store serves a fixture
/// queue so the polymorphic surface renders. Migrating the existing AI-auth and
/// bluetooth modals onto the broker is a later coder step.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The eleven request classes (contracts/consent-contract ConsentClass).
export type ConsentClass =
  | "capability_grant"
  | "app_data"
  | "install"
  | "destructive"
  | "external_send"
  | "network_access"
  | "exec_confined"
  | "elevated_privilege"
  | "portal"
  | "notification_action"
  | "agent_action";

/// Silent (no dialog), Standard, or High-stakes (polymorphic).
export type SeverityTier = "silent" | "standard" | "high_stakes";

/// The user's decision (contracts/consent-contract ConsentOutcome).
export type ConsentOutcome = "allowed_once" | "allowed_remembered" | "denied";

/// Whether the action can be undone (from InverseClass). This gates autonomy:
/// reversible actions carry into autonomous agent use, only the genuinely
/// irreversible confirm per instance.
export type Reversibility = "reversible" | "reversible_with_cost" | "irreversible";

/// The pending request the dialog renders (daemons/consent-broker PendingView).
export interface PendingView {
  id: number;
  /// The attested app id. The shown identity IS the grant recipient.
  requester: string;
  class: ConsentClass;
  tier: SeverityTier;
  /// The risk/outcome, in plain terms (never the raw resource).
  summary: string;
  /// The concrete target (a path, a host, a recipient), if any.
  scope: string | null;
  /// Whether it can be undone - the gate on "remember" + autonomy. (Contract seam:
  /// the broker holds this via InverseClass; PendingView must surface it.)
  reversibility: Reversibility;
}

// One representative request per tier/class so the design language + the
// high-stakes treatments render under vite.
const MOCK_PENDING: PendingView[] = [
  { id: 1, requester: "org.arlen.files", class: "portal", tier: "standard", summary: "open one file you pick", scope: "a single file you choose", reversibility: "reversible" },
  { id: 2, requester: "com.example.notes", class: "capability_grant", tier: "standard", summary: "read your notes and their tags", scope: "your notes", reversibility: "reversible" },
  { id: 3, requester: "org.arlen.files", class: "destructive", tier: "standard", summary: "move 8 files to the Trash", scope: "~/Downloads", reversibility: "reversible" },
  { id: 4, requester: "org.arlen.files", class: "destructive", tier: "high_stakes", summary: "permanently delete 3 files", scope: "~/Documents/old", reversibility: "irreversible" },
  { id: 5, requester: "com.example.mail", class: "external_send", tier: "high_stakes", summary: "send an email on your behalf", scope: "alex@example.com", reversibility: "irreversible" },
  { id: 6, requester: "org.arlen.installd", class: "elevated_privilege", tier: "high_stakes", summary: "install system software with admin rights", scope: "3 packages", reversibility: "reversible_with_cost" },
];

/// The request on screen now, or null when nothing is pending.
export const current = writable<PendingView | null>(null);

let mockIndex = 0;

/// Fetch the front pending request. Live: `consent_fetch`; under vite it serves
/// the current fixture so the surface renders.
export async function pollConsent(): Promise<void> {
  try {
    current.set(await invoke<PendingView | null>("consent_fetch"));
  } catch {
    current.set(MOCK_PENDING[mockIndex % MOCK_PENDING.length]);
  }
}

/// Answer the request and clear it. Live: `consent_resolve`.
export async function resolve(id: number, outcome: ConsentOutcome): Promise<void> {
  current.set(null);
  try {
    await invoke("consent_resolve", { id, outcome });
  } catch {
    // No broker under vite: the optimistic clear stands.
  }
}

/// Dev-only: step to the next fixture request (the screenshot loop).
export function cycleMock(): void {
  mockIndex = (mockIndex + 1) % MOCK_PENDING.length;
  current.set(MOCK_PENDING[mockIndex]);
}
