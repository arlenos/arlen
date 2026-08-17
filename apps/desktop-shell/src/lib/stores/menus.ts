import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";
import { activeWindow } from "./windows.js";
import { makeDisposer } from "./_disposer.js";

export interface MenuItem {
    label: string;
    action: string;
    shortcut?: string;
    disabled?: boolean;
    checked?: boolean;
    type: "item" | "separator" | "submenu";
    children?: MenuItem[];
}

export interface MenuGroup {
    label: string;
    items: MenuItem[];
}

/// All registered app menus, keyed by app_id.
const appMenus = writable<Map<string, MenuGroup[]>>(new Map());

/// The permission id of the currently active window's app, or null.
///
/// Not the window's own app_id. A window announces the id its toolkit sets -
/// `arlen-knowledge`, matching the `.desktop` file - while a menu is registered
/// under the reverse-DNS id the permission system keys on, `dev.arlen.knowledge`.
/// Keying the lookup on the window's id therefore missed every menu ever
/// registered, and the topbar stayed empty for apps that had published one
/// correctly. `resolve_app_id` reads the app index, where the `.desktop` file's
/// `X-Arlen-AppId=` states which permission id the window belongs to.
///
/// Resolution is a backend round trip, so this is a writable the focus
/// subscription fills rather than a derived store. Results are cached per window
/// id, and a reply that arrives after focus has moved on is discarded.
export const activeAppId = writable<string | null>(null);

/// The menu for the currently active app, or null if none registered.
export const activeMenu = derived([appMenus, activeAppId], ([$menus, $id]) =>
    $id ? ($menus.get($id) ?? null) : null,
);

const resolvedIds = new Map<string, string>();
let resolveSeq = 0;

/// Keep `activeAppId` on the focused window's resolved permission id.
function trackActiveAppId(): () => void {
    return activeWindow.subscribe(($w) => {
        const windowId = $w?.app_id ?? null;
        if (!windowId) {
            resolveSeq++;
            activeAppId.set(null);
            return;
        }
        const cached = resolvedIds.get(windowId);
        if (cached !== undefined) {
            resolveSeq++;
            activeAppId.set(cached);
            return;
        }
        const seq = ++resolveSeq;
        void invoke<string>("resolve_app_id", { windowAppId: windowId })
            .then((resolved) => {
                resolvedIds.set(windowId, resolved);
                if (seq === resolveSeq) activeAppId.set(resolved);
            })
            .catch(() => {
                // The index could not answer. The window's own id is the honest
                // fallback: it is what an app with no `.desktop` file resolves
                // to anyway, so the menu is missing rather than misattributed.
                if (seq === resolveSeq) activeAppId.set(windowId);
            });
    });
}

let started = false;
let teardown: (() => void) | null = null;

export function initMenuListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

    const pending: Array<Promise<UnlistenFn>> = [
        listen<{ app_id: string; items: MenuGroup[] }>(
            "arlen://menu-registered",
            ({ payload }) => {
                appMenus.update(($m) => {
                    const next = new Map($m);
                    next.set(payload.app_id, payload.items);
                    return next;
                });
            },
        ),
        listen<{ app_id: string }>(
            "arlen://menu-unregistered",
            ({ payload }) => {
                appMenus.update(($m) => {
                    const next = new Map($m);
                    next.delete(payload.app_id);
                    return next;
                });
            },
        ),
    ];

    const disposer = makeDisposer(pending);

    // Pull the focused app's menu from the backend store on every
    // focus-in. The live `arlen://menu-registered` event is one-shot
    // (an app registers its menu once at startup), so on a later
    // focus-in the menu can be absent from `appMenus` - the event may
    // have fired before this listener was installed, or while the app
    // was unfocused. `get_menu` reads the authoritative shell-side
    // store, which holds the menu for the app's whole lifetime, so
    // re-fetching on focus makes the menu reappear whenever a
    // registered app is focused, not only the first time.
    const unsubTrack = trackActiveAppId();
    const unsubActive = activeAppId.subscribe((id) => {
        if (id) void fetchMenu(id);
    });

    teardown = () => {
        unsubActive();
        unsubTrack();
        disposer();
        started = false;
        teardown = null;
    };
    return teardown;
}

/// Dispatch a menu action to the backend.
export async function dispatchMenuAction(appId: string, action: string): Promise<void> {
    await invoke("dispatch_menu_action", { appId, action });
}

/// Fetch the menu for an app (used on initial load or focus change).
export async function fetchMenu(appId: string): Promise<MenuGroup[] | null> {
    const result = await invoke<MenuGroup[] | null>("get_menu", { appId });
    if (result) {
        appMenus.update(($m) => {
            const next = new Map($m);
            next.set(appId, result);
            return next;
        });
    }
    return result;
}
