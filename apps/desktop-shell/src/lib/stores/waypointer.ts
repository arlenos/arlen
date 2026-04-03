import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writable } from "svelte/store";

export const waypointerVisible = writable(false);

export function initWaypointerListeners() {
    listen("lunaris://waypointer-show", () => {
        waypointerVisible.set(true);
    });
    listen("lunaris://waypointer-hide", () => {
        waypointerVisible.set(false);
    });
}

export function openWaypointer() {
    waypointerVisible.set(true);
    invoke("toggle_waypointer");
}

export function closeWaypointer() {
    // Set store immediately so the UI reacts before the Tauri round-trip.
    waypointerVisible.set(false);
    invoke("toggle_waypointer");
}
