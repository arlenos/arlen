import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface ZoomState {
    visible: boolean;
    level: number;
    increment: number;
    movement: number;
}

const INITIAL: ZoomState = {
    visible: false,
    level: 1.0,
    increment: 100,
    movement: 1,
};

export const zoom = writable<ZoomState>(INITIAL);

export function initZoomListeners(): void {
    listen<{ level: number; increment: number; movement: number }>(
        "lunaris://zoom-toolbar-show",
        ({ payload }) => {
            zoom.set({
                visible: true,
                level: payload.level,
                increment: payload.increment,
                movement: payload.movement,
            });
        }
    );

    listen<{ level: number }>(
        "lunaris://zoom-toolbar-update",
        ({ payload }) => {
            zoom.update((z) => ({ ...z, level: payload.level }));
        }
    );

    listen(
        "lunaris://zoom-toolbar-hide",
        () => {
            zoom.set(INITIAL);
        }
    );
}

export async function zoomIncrease(): Promise<void> {
    await invoke("zoom_increase");
}

export async function zoomDecrease(): Promise<void> {
    await invoke("zoom_decrease");
}

export async function zoomClose(): Promise<void> {
    await invoke("zoom_close");
}

export async function zoomSetIncrement(value: number): Promise<void> {
    await invoke("zoom_set_increment", { value });
}

export async function zoomSetMovement(mode: number): Promise<void> {
    await invoke("zoom_set_movement", { mode });
}

export const MOVEMENT_CONTINUOUSLY = 1;
export const MOVEMENT_ON_EDGE = 2;
export const MOVEMENT_CENTERED = 3;

export const INCREMENTS = [25, 50, 100, 150, 200];
