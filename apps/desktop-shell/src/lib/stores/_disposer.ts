/// Helper for turning a set of `tauri listen()` promises into a single
/// disposer. Every `initXListeners()` store-initializer should capture
/// its `Promise<UnlistenFn>[]` at mount time and return a disposer so
/// `+layout.svelte` can call all of them in `onMount`'s cleanup.
///
/// Without this, every HMR-triggered re-mount stacks a fresh copy of
/// each listener and the shell's event dispatch rate climbs linearly
/// with time. See `docs` or the performance analysis for context.

import type { UnlistenFn } from "@tauri-apps/api/event";

/// Wraps a list of pending `listen()` promises into a single-shot
/// cleanup function. Safe to call multiple times — the inner `.then()`
/// only fires once per promise.
export function makeDisposer(
    pending: Array<Promise<UnlistenFn>>,
): () => void {
    let disposed = false;
    return () => {
        if (disposed) return;
        disposed = true;
        for (const p of pending) {
            p.then((fn) => fn()).catch(() => {});
        }
    };
}
