import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";

export interface IndicatorState {
    kind: number;
    edges: number;
    direction: number;
    shortcut1: string;
    shortcut2: string;
}

/** Active indicators keyed by kind (1=stack_hover, 2=swap, 3=resize). */
export const indicators = writable<Map<number, IndicatorState>>(new Map());

export function initIndicatorListeners(): void {
    listen<{ kind: number; edges: number; direction: number; shortcut1: string; shortcut2: string }>(
        "lunaris://indicator-show",
        ({ payload }) => {
            indicators.update((m) => {
                m.set(payload.kind, payload);
                return new Map(m);
            });
        }
    );

    listen<{ kind: number }>(
        "lunaris://indicator-hide",
        ({ payload }) => {
            indicators.update((m) => {
                m.delete(payload.kind);
                return new Map(m);
            });
        }
    );
}
