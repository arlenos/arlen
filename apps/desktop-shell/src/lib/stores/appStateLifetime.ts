/// Drop what an app published once it has no windows left.
///
/// THE PROBLEM. The shell keeps per-app state - a menu, a shortcut list, a
/// badge, an ambient effect, a toolbar - keyed by the app's permission id, and nothing ever
/// removed an entry except the app itself saying so. An app that crashes, is
/// killed or goes away with the session never gets to say so, and
/// `badges-api.md` FA4 named the gap and deferred it ("stale entries from exited
/// apps clear via the same TTL / process-exit cleanup deferred for all per-app
/// stores").
///
/// THE RULE, and it is the same one the knowledge daemon now uses for an
/// unclosed presence: OBSERVED ABSENCE, not a timer. A TTL invents a constant
/// nobody can defend, is wrong for exactly its length, and cannot tell a quiet
/// app from a gone one. The daemon asks whether the process is there; the shell
/// has no pid, and does not need one - it already knows every toplevel on the
/// machine, so "this app has no windows" is the same question in the shell's own
/// instrument.
///
/// THE CAUTIOUS HALF, and it is what keeps this from eating live state. An app
/// publishes through the bus and maps its window through the compositor, and
/// those are two races: a badge can arrive before the window exists. So state is
/// only dropped for an app whose window this watcher has actually SEEN and then
/// seen go - never for one that has simply not appeared yet.

import { get } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { windows } from "./windows";
import { resolvePermissionId } from "./activeApp";
import { forgetApp, appsWithState } from "./appStateStores";
import { forgetMenu, appsWithMenus } from "./menus";
import { forgetToolbarApp, appsWithToolbar } from "./toolbarStore";

/// Which apps have outlived their windows.
///
/// Pure, because the rule is the whole of it and the rest is plumbing: given
/// what is stored, what has a window, and what has been seen with one, say what
/// to drop. `seen` is carried by the caller and grows as windows appear.
export function outlived(stored: string[], live: Set<string>, seen: Set<string>): string[] {
  return stored.filter((id) => seen.has(id) && !live.has(id));
}

/// Give up what an app published the moment its permissions change.
///
/// The other half of the same idea, and the one a PERSON can reach. Observed
/// absence handles an app that went away; this handles an app that is still
/// there and may no longer do the thing. Revoking `ambient` from an app left its
/// tint on the screen until it exited, which made the grant - the one instrument
/// somebody has for a running effect - not actually take it back.
///
/// Everything that app published goes, not only the surface that changed. The
/// shell does not read capability files and should not start: whatever the app
/// still may publish, it publishes again, and until then the screen holds
/// nothing it cannot account for. The same shape as the knowledge daemon marking
/// an app's grants stale rather than working out which ones survived.
export function initPermissionForget(): () => void {
  let unlisten: UnlistenFn | null = null;
  void listen<{ appId?: string }>("arlen://permission-changed", (e) => {
    const id = e.payload?.appId;
    if (!id) return;
    forgetApp(id);
    forgetMenu(id);
    forgetToolbarApp(id);
  })
    .then((fn) => {
      unlisten = fn;
    })
    .catch(() => {
      // No host: nothing publishes here either.
    });
  return () => unlisten?.();
}

/// Watch the window list and forget what has outlived it.
///
/// Returns the unsubscribe. Resolution from a window's own id to the permission
/// id the state is keyed by goes through `activeApp`, which is the one place
/// that crossing is allowed to happen.
export function initAppStateLifetime(): () => void {
  const seen = new Set<string>();

  const stop = windows.subscribe(($windows) => {
    void (async () => {
      const live = new Set<string>();
      for (const w of $windows) {
        const id = await resolvePermissionId(w.app_id);
        if (id) {
          live.add(id);
          seen.add(id);
        }
      }
      // A window list that resolved to nothing at all is not evidence that every
      // app went away - it is what a shell looks like before the app index has
      // answered anything. Dropping on it would clear the whole board on the
      // first tick.
      if (live.size === 0 && $windows.length > 0) return;
      const stale = outlived(
        [...appsWithState(), ...appsWithMenus(), ...appsWithToolbar()],
        live,
        seen,
      );
      for (const id of stale) {
        forgetApp(id);
        forgetMenu(id);
        forgetToolbarApp(id);
        seen.delete(id);
      }
    })();
  });

  return stop;
}
