import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { writable } from "svelte/store";

export const waypointerVisible = writable(false);

let started = false;
let teardown: (() => void) | null = null;

/// Install Waypointer show/hide listeners. Returns a disposer that
/// cleans up both listeners. Idempotent: calling twice returns the
/// same disposer (so HMR-driven re-init doesn't stack handlers).
export function initWaypointerListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

    const pending: Promise<UnlistenFn>[] = [
        listen("lunaris://waypointer-show", () => {
            waypointerVisible.set(true);
        }),
        listen("lunaris://waypointer-hide", () => {
            waypointerVisible.set(false);
        }),
    ];

    teardown = () => {
        pending.forEach((p) => p.then((fn) => fn()).catch(() => {}));
        started = false;
        teardown = null;
    };
    return teardown;
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
