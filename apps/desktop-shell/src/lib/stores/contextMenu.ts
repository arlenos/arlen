import { writable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { makeDisposer } from "./_disposer.js";

/// Top-level sentinel matching `u32::MAX` sent by the compositor for items
/// that have no parent submenu.
export const TOP_LEVEL_PARENT = 0xffffffff;

export interface MenuItem {
    index: number;
    kind: "entry" | "separator";
    action: number | null;
    label: string | null;
    toggled: boolean | null;
    disabled: boolean | null;
    shortcut: string | null;
    parent_index: number;
    has_submenu: boolean;
}

/// A menu item augmented with its rebuilt children list. Used only for
/// rendering; the flat `MenuItem[]` stays in the store.
export interface MenuItemNode extends MenuItem {
    children: MenuItemNode[];
}

/// Build a tree of `MenuItemNode` from the flat DFS stream. Each item is
/// placed under its parent (keyed by `parent_index`); top-level items
/// (`parent_index === TOP_LEVEL_PARENT`) form the returned root array.
export function buildMenuTree(items: MenuItem[]): MenuItemNode[] {
    const nodes = new Map<number, MenuItemNode>();
    for (const it of items) {
        nodes.set(it.index, { ...it, children: [] });
    }
    const roots: MenuItemNode[] = [];
    for (const it of items) {
        const node = nodes.get(it.index)!;
        if (it.parent_index === TOP_LEVEL_PARENT) {
            roots.push(node);
        } else {
            const parent = nodes.get(it.parent_index);
            if (parent) {
                parent.children.push(node);
            } else {
                // Orphaned child — fall back to top-level so the entry
                // stays clickable instead of vanishing silently.
                roots.push(node);
            }
        }
    }
    return roots;
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

let started = false;
let teardown: (() => void) | null = null;

export function initContextMenuListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

    console.log("[contextMenu] initContextMenuListeners called, registering listeners");

    const showPromise = listen<{ menu_id: number; x: number; y: number; items: MenuItem[] }>(
        "lunaris://context-menu-show",
        ({ payload }) => {
            console.log("[contextMenu] context-menu-show received:", payload);
            contextMenu.set({ visible: true, ...payload });
        },
    );
    showPromise.then(() => {
        console.log("[contextMenu] context-menu-show listener registered");
    }).catch((e) => {
        console.error("[contextMenu] failed to register context-menu-show listener:", e);
    });

    const hidePromise = listen<{ menu_id: number }>(
        "lunaris://context-menu-hide",
        ({ payload }) => {
            console.log("[contextMenu] context-menu-hide received:", payload);
            contextMenu.set(HIDDEN);
        },
    );
    hidePromise.catch((e) => {
        console.error("[contextMenu] failed to register context-menu-hide listener:", e);
    });

    const pending: Array<Promise<UnlistenFn>> = [showPromise, hidePromise];
    const disposer = makeDisposer(pending);
    teardown = () => { disposer(); started = false; teardown = null; };
    return teardown;
}
