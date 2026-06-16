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

/// The menu for the currently active app, or null if none registered.
export const activeMenu = derived(
    [appMenus, activeWindow],
    ([$menus, $active]) => {
        if (!$active) return null;
        return $menus.get($active.app_id) ?? null;
    }
);

/// The app_id of the currently active window.
export const activeAppId = derived(activeWindow, ($w) => $w?.app_id ?? null);

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
    const unsubActive = activeAppId.subscribe((id) => {
        if (id) void fetchMenu(id);
    });

    teardown = () => {
        unsubActive();
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
