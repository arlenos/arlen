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
  /// External-send only: the named recipient the data leaves Arlen to.
  recipient?: string;
  /// External-send only: a short preview of the content that would leave Arlen,
  /// so "send once" is an informed decision, not a blind one.
  preview?: string;
  /// Destructive only: the concrete items and their sizes. Names what is lost.
  targets?: { name: string; size: string }[];
  /// Destructive only: the total size affected, shown beside the target.
  total?: string;
  /// True when an external document or site triggered this, not the user
  /// directly - the surface warns before a standing grant is spent unattended.
  triggeredExternally?: boolean;
}

// One representative request per tier/class so the design language + the
// high-stakes treatments render under vite.
const MOCK_PENDING: PendingView[] = [
  { id: 1, requester: "org.arlen.files", class: "portal", tier: "standard", summary: "open one file you pick", scope: "a single file you choose", reversibility: "reversible" },
  { id: 2, requester: "com.example.notes", class: "capability_grant", tier: "standard", summary: "read your notes and their tags", scope: "your notes", reversibility: "reversible" },
  { id: 3, requester: "org.arlen.files", class: "destructive", tier: "standard", summary: "move 8 files to the Trash", scope: "~/Downloads", reversibility: "reversible" },
  { id: 4, requester: "org.arlen.files", class: "destructive", tier: "high_stakes", summary: "permanently delete 3 files", scope: "~/Documents/old", reversibility: "irreversible", total: "1.2 GB", targets: [
    { name: "report-final.pdf", size: "840 MB" },
    { name: "archive-2025.zip", size: "360 MB" },
    { name: "notes.md", size: "4 KB" },
  ] },
  { id: 5, requester: "com.example.mail", class: "external_send", tier: "high_stakes", summary: "send an email on your behalf", scope: "alex@example.com", reversibility: "irreversible", recipient: "alex@example.com", preview: "Subject: Re: Thursday\n\"Sounds good, see you at 3. I'll bring the printouts.\"" },
  { id: 6, requester: "org.arlen.installd", class: "elevated_privilege", tier: "high_stakes", summary: "install system software with admin rights", scope: "3 packages", reversibility: "reversible_with_cost" },
];

/// The request on screen now, or null when nothing is pending.
export const current = writable<PendingView | null>(null);

let mockIndex = 0;

/// Fetch the front pending request. Live: `consent_fetch`. When no broker
/// answers, the fixture is served ONLY under vite (dev) so the surface renders
/// for screenshots; on a real boot a broker failure shows nothing rather than
/// covering the desktop with a mock request every session.
export async function pollConsent(): Promise<void> {
  try {
    current.set(await invoke<PendingView | null>("consent_fetch"));
  } catch {
    current.set(import.meta.env.DEV ? MOCK_PENDING[mockIndex % MOCK_PENDING.length] : null);
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
