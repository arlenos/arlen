import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface WindowHeaderState {
    surface_id: number;
    x: number;
    y: number;
    width: number;
    height: number;
    title: string;
    activated: boolean;
    has_minimize: boolean;
    has_maximize: boolean;
}

export const windowHeaders = writable<Map<number, WindowHeaderState>>(new Map());

export const HEADER_ACTION_MINIMIZE = 1;
export const HEADER_ACTION_MAXIMIZE = 2;
export const HEADER_ACTION_CLOSE = 3;
export const HEADER_ACTION_MOVE = 4;

export async function headerAction(surfaceId: number, action: number): Promise<void> {
    await invoke("window_header_action", { surfaceId, action });
}

export function initWindowHeaderListeners(): void {
    listen<{
        surface_id: number; x: number; y: number; width: number; height: number;
        title: string; activated: boolean; has_minimize: boolean; has_maximize: boolean;
    }>(
        "lunaris://window-header-show",
        ({ payload }) => {
            windowHeaders.update((m) => {
                m.set(payload.surface_id, payload);
                return new Map(m);
            });
        }
    );

    listen<{
        surface_id: number; x: number; y: number; width: number; height: number;
        title: string; activated: boolean;
    }>(
        "lunaris://window-header-update",
        ({ payload }) => {
            windowHeaders.update((m) => {
                const existing = m.get(payload.surface_id);
                if (existing) {
                    m.set(payload.surface_id, {
                        ...existing,
                        x: payload.x,
                        y: payload.y,
                        width: payload.width,
                        height: payload.height,
                        title: payload.title,
                        activated: payload.activated,
                    });
                }
                return new Map(m);
            });
        }
    );

    listen<{ surface_id: number }>(
        "lunaris://window-header-hide",
        ({ payload }) => {
            windowHeaders.update((m) => {
                m.delete(payload.surface_id);
                return new Map(m);
            });
        }
    );
}
