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
//
// i18n-foreign: each `summary` stands in for a sentence the REQUESTING APP
// sends over the wire (`RequestBody.summary`), so it is not ours to translate
// and the shell cannot translate it either - it arrives as prose, in whatever
// language that app was written in. Which means the one dialog a user must
// understand to answer safely is the one sentence we cannot put in their
// language. That is a contract question rather than a fixture question, and it
// is worse than an i18n gap: an app also chooses its own wording for a security
// prompt. Recorded in coder-reports.md, 7 August.
const MOCK_PENDING: PendingView[] = [
  { id: 1, requester: "org.arlen.files", class: "portal", tier: "standard", summary: "open one file you pick", scope: null, reversibility: "reversible" },
  { id: 2, requester: "com.example.notes", class: "capability_grant", tier: "standard", summary: "read your notes and their tags", scope: "your notes", reversibility: "reversible" },
  { id: 3, requester: "org.arlen.files", class: "destructive", tier: "standard", summary: "move 8 files to the Trash", scope: "~/Downloads", reversibility: "reversible" },
  { id: 4, requester: "org.arlen.files", class: "destructive", tier: "high_stakes", summary: "permanently delete 3 files", scope: "~/Documents/old", reversibility: "irreversible", total: "1.2 GB", targets: [
    { name: "report-final.pdf", size: "840 MB" },
    { name: "archive-2025.zip", size: "360 MB" },
    { name: "notes.md", size: "4 KB" },
  ] },
  { id: 5, requester: "com.example.mail", class: "external_send", tier: "high_stakes", summary: "send an email on your behalf", scope: "alex@example.com", reversibility: "irreversible", recipient: "alex@example.com", preview: "Subject: Re: Thursday\n\"Sounds good, see you at 3. I'll bring the printouts.\"" },
  { id: 6, requester: "org.arlen.installd", class: "elevated_privilege", tier: "high_stakes", summary: "install system software with admin rights", scope: "3 packages", reversibility: "reversible_with_cost" },
  { id: 7, requester: "com.example.notes", class: "network_access", tier: "standard", summary: "connect to its sync service", scope: "sync.example.com", reversibility: "reversible", triggeredExternally: true },
];

/// The request on screen now, or null when nothing is pending.
export const current = writable<PendingView | null>(null);

// The shell's input region follows the CARD, not the request, and the dialog
// component owns it (see ConsentDialog). The card is a centered modal in a window
// whose default region is the top bar only, so without expanding it the dialog is
// visible but click-through and a click on Allow lands on the desktop.
//
// It used to be driven from here, off this store, which meant the region and the
// keyboard grab were dropped the instant a request was answered - while the card
// was still fading. Ownership belongs with whatever knows when the card is really
// gone, and that is the component.

// `?consentmock=<n>` (DEV only) pins which fixture request renders, so the
// screenshot loop can address every state by URL; without it the first one
// shows. Same pattern as the waypointer's `?askmock`.
let mockIndex = 0;
if (import.meta.env.DEV && typeof location !== "undefined") {
  const pinned = Number(new URLSearchParams(location.search).get("consentmock"));
  if (Number.isInteger(pinned) && pinned >= 0) mockIndex = pinned;
}

/// Fetch the front pending request. Live: `consent_fetch`. When no broker
/// answers, the fixture is served ONLY under vite (dev) so the surface renders
/// for screenshots; on a real boot a broker failure shows nothing rather than
/// covering the desktop with a mock request every session.
/// Requests this shell has already answered, so a poll reply that was in flight
/// when the user clicked cannot put the dialog back.
///
/// The poll runs every second and `resolve` clears the dialog optimistically, so
/// a fetch issued a moment BEFORE the click returns a still-pending view a moment
/// after it and re-sets the store - the dialog reappears for up to a second,
/// looking exactly like an answer that did not take. Cleared whenever the broker
/// says nothing is pending, so it cannot grow.
const answered = new Set<number>();

export async function pollConsent(): Promise<void> {
  try {
    const view = await invoke<PendingView | null>("consent_fetch");
    if (view === null) {
      answered.clear();
      current.set(null);
      return;
    }
    if (answered.has(view.id)) {
      return; // A reply that predates our answer; the next poll has the truth.
    }
    current.set(view);
  } catch {
    current.set(import.meta.env.DEV ? MOCK_PENDING[mockIndex % MOCK_PENDING.length] : null);
  }
}

/// Answer the request and clear it. Live: `consent_resolve`.
///
/// The optimistic clear makes the dialog feel instant, but it also means a
/// FAILED resolve is invisible: the request is still pending in the broker, the
/// next poll fetches it again, and the dialog reappears looking like it was never
/// answered. That is precisely the boot symptom - a dialog that "stands" while the
/// broker logs a queued request and no verdict - and swallowing the error is what
/// made it undiagnosable. It is logged now, with the id and the reason, so the
/// journal says whether the click reached the broker and what it answered.
///
/// The error is not surfaced to the user, because the reappearing dialog is the
/// signal that the answer did not take and there is nothing else useful for them
/// to do with it. That safety net only works if the failure path gives it back:
/// `answered` is what tells `poll` to ignore a request it has already replied to,
/// so a failed resolve that left the id in there would suppress the dialog
/// FOREVER - the broker keeps the request pending, every later poll matches
/// `answered.has(view.id)` and returns early, and the one thing the user could
/// have done about it never appears again. So the catch takes the marker back
/// out, and the next poll presents the request as unanswered, which it is.
export async function resolve(id: number, outcome: ConsentOutcome): Promise<void> {
  answered.add(id);
  current.set(null);
  try {
    // The Rust `ConsentOutcome` is `#[serde(tag = "outcome")]`, so it
    // deserializes from `{"outcome": "allowed_once"}` and NOT from the bare
    // string. Sending the string made Tauri fail to deserialize the argument and
    // never call the command at all - no broker request, no Rust log, and the
    // dialog reappearing on the next poll, which is what "the dialog stands" was.
    await invoke("consent_resolve", { id, outcome: { outcome } });
  } catch (e) {
    // Under vite there is no broker and this is expected; on a real session it
    // means the answer did not reach the broker.
    answered.delete(id);
    console.error(`[consent] resolve failed for id=${id} outcome=${outcome}:`, e);
  }
}

/// Dev-only: step to the next fixture request (the screenshot loop).
export function cycleMock(): void {
  mockIndex = (mockIndex + 1) % MOCK_PENDING.length;
  current.set(MOCK_PENDING[mockIndex]);
}
