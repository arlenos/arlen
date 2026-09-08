/// What one app has actually DONE, beside what it may reach.
///
/// The app page has always shown the declared half - the grants, the scopes it
/// holds - under a line saying whether it used them "isn't measured yet". It is
/// measured: every gated action is appended to the audit ledger with the actor
/// the kernel attested, and `settings_app_audit` reads it filtered to one app.
/// The command was registered and nothing asked, so the page kept apologising
/// for a record that was sitting there.
///
/// Structural tier only, which is what makes this safe to show on a settings
/// page: kinds, coarse subjects and outcomes, never the content of anything.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One ledger entry as the page renders it (the Rust `ActivityEntry`, trimmed
/// to the fields a per-app view uses).
export interface AuditEntry {
  index: number;
  timestampMicros: number;
  /// Kebab-case kind token - `graph-access`, `tool-call`. The page words it;
  /// the wire spelling is snake_case and matches nothing here.
  kind: string;
  subject: string;
  outcome: string;
}

/// A page of one app's history, plus the two facts a reader needs about the
/// record itself.
export interface AuditPage {
  entries: AuditEntry[];
  /// The daemon answered. False is not "nothing happened".
  available: boolean;
  /// The daemon reports its own ledger unverifiable.
  tampered: boolean;
  /// How many entries this app has, which is more than the page shows.
  total: number;
}

/// How many rows the section lists. A settings page is not a log viewer: enough
/// to see the shape of what an app does, and the total says how much is behind
/// it.
const PAGE = 20;

const _page = writable<AuditPage | null>(null);
export const appAudit: Readable<AuditPage | null> = { subscribe: _page.subscribe };

/// Read one app's history. Null while it is being read.
export async function loadAppAudit(appId: string): Promise<void> {
  _page.set(null);
  try {
    _page.set(await invoke<AuditPage>("settings_app_audit", { appId, limit: PAGE }));
  } catch (e) {
    // The command itself did not answer, which is the same fact for a reader as
    // the daemon not answering: the record cannot be shown. Never an empty list,
    // which would say this app has done nothing.
    console.warn("[settings] app audit read failed:", e);
    _page.set({ entries: [], available: false, tampered: false, total: 0 });
  }
}
