import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { writable, derived } from "svelte/store";
import { activeWindow } from "./windows.js";

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

export function initMenuListeners() {
    listen<{ app_id: string; items: MenuGroup[] }>("lunaris://menu-registered", ({ payload }) => {
        appMenus.update(($m) => {
            const next = new Map($m);
            next.set(payload.app_id, payload.items);
            return next;
        });
    });

    listen<{ app_id: string }>("lunaris://menu-unregistered", ({ payload }) => {
        appMenus.update(($m) => {
            const next = new Map($m);
            next.delete(payload.app_id);
            return next;
        });
    });
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
