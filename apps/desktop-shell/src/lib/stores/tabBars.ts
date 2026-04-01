import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface TabInfo {
    index: number;
    title: string;
    app_id: string;
}

export interface TabBarState {
    stack_id: number;
    x: number;
    y: number;
    width: number;
    height: number;
    active: number;
    tabs: TabInfo[];
}

export const tabBars = writable<Map<number, TabBarState>>(new Map());

export async function activateTab(stackId: number, index: number): Promise<void> {
    await invoke("tab_activate", { stackId, index });
}

export function initTabBarListeners(): void {
    listen<{ stack_id: number; x: number; y: number; width: number; height: number }>(
        "lunaris://tab-bar-show",
        ({ payload }) => {
            tabBars.update((bars) => {
                const existing = bars.get(payload.stack_id);
                bars.set(payload.stack_id, {
                    stack_id: payload.stack_id,
                    x: payload.x,
                    y: payload.y,
                    width: payload.width,
                    height: payload.height,
                    active: existing?.active ?? 0,
                    tabs: existing?.tabs ?? [],
                });
                return new Map(bars);
            });
        }
    );

    listen<{ stack_id: number }>(
        "lunaris://tab-bar-hide",
        ({ payload }) => {
            tabBars.update((bars) => {
                bars.delete(payload.stack_id);
                return new Map(bars);
            });
        }
    );

    listen<{ stack_id: number; index: number; title: string; app_id: string; active: boolean }>(
        "lunaris://tab-added",
        ({ payload }) => {
            tabBars.update((bars) => {
                const bar = bars.get(payload.stack_id);
                if (bar) {
                    bar.tabs.splice(payload.index, 0, {
                        index: payload.index,
                        title: payload.title,
                        app_id: payload.app_id,
                    });
                    // Re-index after splice.
                    bar.tabs.forEach((t, i) => (t.index = i));
                    if (payload.active) bar.active = payload.index;
                } else {
                    // Stack not yet shown via tab_bar_show; create placeholder.
                    bars.set(payload.stack_id, {
                        stack_id: payload.stack_id,
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 24,
                        active: payload.active ? payload.index : 0,
                        tabs: [{ index: payload.index, title: payload.title, app_id: payload.app_id }],
                    });
                }
                return new Map(bars);
            });
        }
    );

    listen<{ stack_id: number; index: number }>(
        "lunaris://tab-removed",
        ({ payload }) => {
            tabBars.update((bars) => {
                const bar = bars.get(payload.stack_id);
                if (bar) {
                    bar.tabs.splice(payload.index, 1);
                    bar.tabs.forEach((t, i) => (t.index = i));
                    if (bar.active >= bar.tabs.length && bar.tabs.length > 0) {
                        bar.active = bar.tabs.length - 1;
                    }
                    if (bar.tabs.length === 0) {
                        bars.delete(payload.stack_id);
                    }
                }
                return new Map(bars);
            });
        }
    );

    listen<{ stack_id: number; index: number }>(
        "lunaris://tab-activated",
        ({ payload }) => {
            tabBars.update((bars) => {
                const bar = bars.get(payload.stack_id);
                if (bar) bar.active = payload.index;
                return new Map(bars);
            });
        }
    );

    listen<{ stack_id: number; index: number; title: string }>(
        "lunaris://tab-title-changed",
        ({ payload }) => {
            tabBars.update((bars) => {
                const bar = bars.get(payload.stack_id);
                if (bar) {
                    const tab = bar.tabs.find((t) => t.index === payload.index);
                    if (tab) tab.title = payload.title;
                }
                return new Map(bars);
            });
        }
    );
}
