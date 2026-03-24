import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface MenuItem {
    index: number;
    kind: "entry" | "separator";
    action: number | null;
    label: string | null;
    toggled: boolean | null;
    disabled: boolean | null;
    shortcut: string | null;
}

interface ContextMenuState {
    visible: boolean;
    menu_id: number;
    x: number;
    y: number;
    items: MenuItem[];
}

const HIDDEN: ContextMenuState = { visible: false, menu_id: 0, x: 0, y: 0, items: [] };

export const contextMenu = writable<ContextMenuState>(HIDDEN);

export async function activateItem(menu_id: number, index: number): Promise<void> {
    await invoke("context_menu_activate", { menuId: menu_id, index });
    contextMenu.set(HIDDEN);
}

export async function dismissMenu(menu_id: number): Promise<void> {
    await invoke("context_menu_dismiss", { menuId: menu_id });
    contextMenu.set(HIDDEN);
}

export function initContextMenuListeners(): void {
    console.log("[contextMenu] initContextMenuListeners called, registering listeners");
    listen<{ menu_id: number; x: number; y: number; items: MenuItem[] }>(
        "lunaris://context-menu-show",
        ({ payload }) => {
            console.log("[contextMenu] context-menu-show received:", payload);
            contextMenu.set({ visible: true, ...payload });
        }
    ).then(() => {
        console.log("[contextMenu] context-menu-show listener registered");
    }).catch((e) => {
        console.error("[contextMenu] failed to register context-menu-show listener:", e);
    });
    listen<{ menu_id: number }>(
        "lunaris://context-menu-hide",
        ({ payload }) => {
            console.log("[contextMenu] context-menu-hide received:", payload);
            contextMenu.set(HIDDEN);
        }
    ).catch((e) => {
        console.error("[contextMenu] failed to register context-menu-hide listener:", e);
    });
}
