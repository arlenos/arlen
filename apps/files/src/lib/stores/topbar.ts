/// The global-topbar producer (file-manager-plan.md §surfaces,
/// topbar-toolbar.md): under the Arlen shell the FM pushes its
/// breadcrumb into the topbar's toolbar slot and routes clicks back
/// to navigation; the local toolbar row stays only as the fallback
/// outside the shell. The slot renders ONE variant at a time
/// (contract: variants are mutually exclusive), so the breadcrumb —
/// the FM's identity — wins over quick actions; view/search/info ride
/// on shortcuts and the fallback. Flagged: the FM doc asks for both.

import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { toolbar, type BreadcrumbItem } from "@arlen/tauri-plugin-shell";
import { breadcrumb, type BrowserState } from "@arlen/ui-kit/components/browser";
import { tauriAvailable } from "$lib/tauri";
import { focusedController } from "$lib/stores/panes";
import { homePath } from "$lib/stores/places";

/// True when the app runs under the Arlen shell: the topbar carries
/// the chrome and the local toolbar hides.
export const shellPresent = writable(false);

const NAV_PREFIX = "nav:";

// Idempotent: the layout calls this on mount; HMR or a remount must
// not double the subscriptions.
let started = false;

function crumbItems(path: string): BreadcrumbItem[] {
  const home = get(homePath);
  let crumbs = breadcrumb(path);
  if (home && (path === home || path.startsWith(home + "/"))) {
    const homeCrumbs = breadcrumb(home);
    crumbs = [{ name: "Home", path: home }, ...crumbs.slice(homeCrumbs.length)];
  }
  return crumbs.map((c) => ({ label: c.name, action: NAV_PREFIX + c.path }));
}

/// Start the topbar sync once from the layout. No-op outside Tauri;
/// outside the shell it probes once and stays silent.
export async function initTopbar(): Promise<void> {
  if (started || !tauriAvailable) return;
  started = true;
  try {
    shellPresent.set(await invoke<boolean>("shell_present"));
  } catch {
    shellPresent.set(false);
  }
  if (!get(shellPresent)) return;

  // Follow the focused pane's location; a late homePath (places
  // load) re-collapses the crumbs.
  let unPath: (() => void) | null = null;
  const push = () => {
    const c = get(focusedController);
    if (c) void toolbar.setBreadcrumb(crumbItems(get(c.path)));
  };
  focusedController.subscribe((c) => {
    unPath?.();
    unPath = null;
    if (!c) return;
    unPath = c.path.subscribe(() => push());
  });
  homePath.subscribe(() => push());

  await toolbar.onAction(({ action }) => {
    if (action.startsWith(NAV_PREFIX)) {
      const target = action.slice(NAV_PREFIX.length);
      void get(focusedController)?.navigate(target);
    }
  });
}
