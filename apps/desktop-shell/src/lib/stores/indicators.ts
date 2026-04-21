import { writable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { makeDisposer } from "./_disposer.js";

export interface IndicatorState {
    kind: number;
    edges: number;
    direction: number;
    shortcut1: string;
    shortcut2: string;
}

/** Active indicators keyed by kind (1=stack_hover, 2=swap, 3=resize). */
export const indicators = writable<Map<number, IndicatorState>>(new Map());

let started = false;
let teardown: (() => void) | null = null;

export function initIndicatorListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

    const pending: Array<Promise<UnlistenFn>> = [
        listen<{ kind: number; edges: number; direction: number; shortcut1: string; shortcut2: string }>(
            "lunaris://indicator-show",
            ({ payload }) => {
                indicators.update((m) => {
                    m.set(payload.kind, payload);
                    return new Map(m);
                });
            },
        ),
        listen<{ kind: number }>(
            "lunaris://indicator-hide",
            ({ payload }) => {
                indicators.update((m) => {
                    m.delete(payload.kind);
                    return new Map(m);
                });
            },
        ),
    ];

    const disposer = makeDisposer(pending);
    teardown = () => { disposer(); started = false; teardown = null; };
    return teardown;
}
