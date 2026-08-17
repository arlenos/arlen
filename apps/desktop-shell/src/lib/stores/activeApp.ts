/**
 * The permission id of the focused window's app.
 *
 * The shell knows an app by two names. A window announces the id its toolkit
 * sets, which matches its `.desktop` file - `arlen-knowledge`. Everything an app
 * publishes over the Event Bus is keyed by the id the permission system resolves
 * from the calling process - `dev.arlen.knowledge`. Both are correct in their own
 * domain, and neither can be renamed into the other: a third-party window
 * announces whatever its toolkit sets, and will never carry one of our
 * reverse-DNS ids.
 *
 * So anything that pairs "what the focused window is" with "what an app
 * published" has to cross between the two, and every surface that did it by
 * hand got it wrong the same way - menus, the toolbar, shortcuts, badges and
 * ambient all looked their state up under the window's id and found nothing,
 * which renders as a shell that shows no menu and an empty toolbar for apps that
 * published both correctly. This module exists so that crossing happens once.
 *
 * `resolve_app_id` answers from the app index, where the `.desktop` file's
 * `X-Arlen-AppId=` states which permission id an app's windows belong to.
 */

import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";
import { activeWindow } from "./windows";

/// The focused window's app, as the permission system names it, or null when
/// nothing is focused. Populated by a backend round trip, so it lags focus by
/// one tick on the first sighting of an app and is immediate afterwards.
export const activeAppId = writable<string | null>(null);

/// Window id to permission id. An app's answer never changes while the shell
/// runs: it comes from a `.desktop` file read at index build.
const resolved = new Map<string, string>();

/// Guards against a slow reply landing after focus has moved on. Every focus
/// change takes a number; a reply that is not the current one is dropped.
let seq = 0;

/// Resolve on focus. Subscribed at module scope, for the lifetime of the shell:
/// the stores derived from this are read by the top bar from first paint, and a
/// caller that forgot to start the tracking would see the same empty surfaces
/// this module exists to fix.
activeWindow.subscribe(($w) => {
  const windowId = $w?.app_id ?? null;
  if (!windowId) {
    seq++;
    activeAppId.set(null);
    return;
  }
  const known = resolved.get(windowId);
  if (known !== undefined) {
    seq++;
    activeAppId.set(known);
    return;
  }
  const mine = ++seq;
  void invoke<string>("resolve_app_id", { windowAppId: windowId })
    .then((id) => {
      resolved.set(windowId, id);
      if (mine === seq) activeAppId.set(id);
    })
    .catch(() => {
      // The index could not answer. The window's own id is the honest fallback:
      // it is what an app with no `.desktop` file resolves to anyway, so state
      // goes missing rather than being attributed to the wrong app.
      if (mine === seq) activeAppId.set(windowId);
    });
});
