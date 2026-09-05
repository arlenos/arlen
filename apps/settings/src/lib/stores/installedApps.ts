/// The installed apps, as the row source for the per-app settings list.
///
/// WHY THIS EXISTS RATHER THAN THE GRANT LEDGER. The list used to be derived
/// from `access_grants`, which meant an app appeared only once it held a grant.
/// `settings_apps_list` says the reason that is wrong in its own doc: an app that
/// ships a settings schema and holds no grant still has settings, and a page you
/// cannot reach is indistinguishable from an app that has none. So installed
/// entries are what make a row exist, and a grant is a property OF a row.
///
/// The union is deliberate rather than a straight swap. An app can hold a live
/// grant and ship no desktop entry - a background principal, or something
/// installed outside the entry directories - and dropping it would take a
/// reachable page away to fix a missing one. Nothing that appears today
/// disappears.

import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";
import { tauriAvailable } from "$lib/tauri";

/// One installed app as the host reports it, straight off its desktop entry.
export interface AppRow {
  app_id: string;
  name: string;
  version: string | null;
  publisher: string | null;
}

/// A row the list renders: an app, and whether the ledger knows anything of it.
export interface ListedApp {
  appId: string;
  label: string;
  /// False only when a grant says so. An app with no grant is not "unverified";
  /// nothing has claimed anything about it, which is a different sentence.
  identityVerified: boolean;
  /// Whether this row came from an installed entry, a grant, or both. Rendered
  /// nowhere yet; it is here so the list can say why a row is present when that
  /// question comes up, rather than the answer having to be re-derived.
  source: "installed" | "granted" | "both";
}

/// The principal shape this merges against, narrowed to what it reads.
export interface GrantedPrincipal {
  appId: string;
  label: string;
  identityVerified: boolean;
}

export const installed = writable<AppRow[]>([]);
export const installedLoaded = writable(false);
export const installedError = writable(false);

/// Read the installed apps. A failure is recorded, never answered with a list:
/// an empty page and a page that could not ask are different facts and the
/// surface says which.
///
/// Under `vite` there is no host to ask, which is NOT a failure - it is the
/// design-time state, and the page already says it is showing samples because
/// the grant store labels itself. Reporting it as an error there would put a
/// fault line on a screen where nothing is wrong. There is deliberately no
/// fixture list of installed apps: inventing what is on somebody's machine is
/// the one thing this page must not do, so with no host it lists nothing and
/// lets the grant sample carry the surface.
export async function loadInstalledApps(): Promise<void> {
  if (!tauriAvailable) {
    installed.set([]);
    installedError.set(false);
    installedLoaded.set(true);
    return;
  }
  try {
    installed.set(await invoke<AppRow[]>("settings_apps_list"));
    installedError.set(false);
  } catch {
    installed.set([]);
    installedError.set(true);
  } finally {
    installedLoaded.set(true);
  }
}

/// Merge installed entries with the principals the ledger knows.
///
/// Installed entries win on the label, because a desktop entry's `Name` is what
/// the app calls itself and `principalLabel` is a derivation from an id.
export function mergeAppRows(
  rows: AppRow[],
  granted: GrantedPrincipal[],
  collator?: Intl.Collator,
): ListedApp[] {
  const out = new Map<string, ListedApp>();
  for (const r of rows) {
    out.set(r.app_id, {
      appId: r.app_id,
      label: r.name || r.app_id,
      identityVerified: true,
      source: "installed",
    });
  }
  for (const g of granted) {
    const seen = out.get(g.appId);
    if (seen) {
      seen.identityVerified = g.identityVerified;
      seen.source = "both";
    } else {
      out.set(g.appId, {
        appId: g.appId,
        label: g.label,
        identityVerified: g.identityVerified,
        source: "granted",
      });
    }
  }
  const list = [...out.values()];
  const cmp = collator ?? new Intl.Collator();
  return list.sort((a, b) => cmp.compare(a.label, b.label));
}
