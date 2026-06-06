import { writable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { makeDisposer } from "./_disposer.js";

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
    /// Nonzero = this header belongs to a CosmicStack and should
    /// render tabs (correlated via the `tabBars` store on the same
    /// stack_id). Zero = plain header with title only.
    stack_id: number;
}

export const windowHeaders = writable<Map<number, WindowHeaderState>>(new Map());

/// Set of surface_ids currently in an interactive drag/resize grab.
/// The compositor emits `window_drag_start` when a grab installs
/// and `window_drag_end` when it releases — matching pairs
/// (Feature 4). WindowHeader.svelte reads this store to disable
/// CSS transitions on the active header so translate3d() updates
/// don't fight a 100ms transform tween.
export const draggingSurfaces = writable<Set<number>>(new Set());

export const HEADER_ACTION_MINIMIZE = 1;
export const HEADER_ACTION_MAXIMIZE = 2;
export const HEADER_ACTION_CLOSE = 3;
export const HEADER_ACTION_MOVE = 4;

export async function headerAction(surfaceId: number, action: number): Promise<void> {
    await invoke("window_header_action", { surfaceId, action });
}

/// Sync the current header rectangles to the Rust side so the
/// GTK layer-surface's input-region can include them. Without this,
/// the shell layer is click-through everywhere outside the top-bar
/// and buttons on Lunaris-rendered window headers wouldn't receive
/// pointer events. Debounced via microtask so a batch of show/
/// update/hide events produces ONE backend call.
///
/// Critically, we only include the BUTTON area (right-aligned ~100px)
/// in the input region — NOT the full header. The title-drag area
/// on the left must stay click-through so the compositor's native
/// SSD PointerTarget routing can start interactive move. With the
/// whole header included, every click on the title area got
/// swallowed by the shell layer-surface → move was impossible.
let _syncScheduled = false;

/// Width of the button strip on the right of each header. Derived
/// from the CSS: `.header-btn` is 28px wide, no gap, 4px right pad.
/// 3 buttons (min + max + close) = 84 + 4 = 88px, but we round up
/// for hover tolerance. 2 buttons (no min or no max) use the same
/// generous budget — over-inclusion means the button-area input-
/// region covers blank space too, which is harmless.
const HEADER_BUTTON_AREA_W = 96;

function scheduleHeaderRegionSync(current: Map<number, WindowHeaderState>): void {
    if (_syncScheduled) return;
    _syncScheduled = true;
    queueMicrotask(() => {
        _syncScheduled = false;
        const rects: Array<[number, number, number, number]> = [];
        for (const h of current.values()) {
            // Button strip (right-aligned) — always clickable.
            const btnW = Math.min(HEADER_BUTTON_AREA_W, h.width);
            const btnX = h.x + Math.max(0, h.width - btnW);
            rects.push([btnX, h.y, btnW, h.height]);

            // Stacked header (Feature 3) — tabs on the left must
            // also capture clicks so users can switch tabs. The
            // title/drag area between tabs and buttons stays
            // fall-through so the compositor keeps handling the
            // title drag via its native SSD routing.
            if (h.stack_id !== 0) {
                const tabsW = Math.max(0, h.width - btnW);
                if (tabsW > 0) {
                    // Conservative estimate: reserve the left 60% of
                    // the non-button strip for tabs. The exact number
                    // depends on dynamic tab-bar layout; erring on the
                    // side of too-wide keeps tab clicks working even
                    // when many tabs are open, and only costs us a
                    // bit of drag area which is still fine because
                    // grabbing the title anywhere starts a move.
                    const tabAreaW = Math.min(tabsW, Math.floor(h.width * 0.6));
                    rects.push([h.x, h.y, tabAreaW, h.height]);
                }
            }
        }
        invoke("update_window_header_regions", { rects }).catch((e) =>
            console.warn("update_window_header_regions failed:", e),
        );
    });
}

let started = false;
let teardown: (() => void) | null = null;

export function initWindowHeaderListeners(): () => void {
    if (started && teardown) return teardown;
    started = true;

    const pending: Array<Promise<UnlistenFn>> = [
        listen<{
            surface_id: number; x: number; y: number; width: number; height: number;
            title: string; activated: boolean; has_minimize: boolean; has_maximize: boolean;
            stack_id: number;
        }>(
            "lunaris://window-header-show",
            ({ payload }) => {
                windowHeaders.update((m) => {
                    m.set(payload.surface_id, payload);
                    const next = new Map(m);
                    scheduleHeaderRegionSync(next);
                    return next;
                });
            },
        ),
        listen<{
            surface_id: number; x: number; y: number; width: number; height: number;
            title: string; activated: boolean; stack_id: number;
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
                            stack_id: payload.stack_id,
                        });
                    }
                    const next = new Map(m);
                    scheduleHeaderRegionSync(next);
                    return next;
                });
            },
        ),
        listen<{ surface_id: number }>(
            "lunaris://window-header-hide",
            ({ payload }) => {
                windowHeaders.update((m) => {
                    m.delete(payload.surface_id);
                    const next = new Map(m);
                    scheduleHeaderRegionSync(next);
                    return next;
                });
                // Clear any leftover drag flag — if the window
                // vanished mid-drag the compositor's drag_end will
                // arrive too, but being defensive here avoids a
                // stuck entry.
                draggingSurfaces.update((s) => {
                    if (s.has(payload.surface_id)) {
                        const next = new Set(s);
                        next.delete(payload.surface_id);
                        return next;
                    }
                    return s;
                });
            },
        ),
        listen<{ surface_id: number }>(
            "lunaris://window-drag-start",
            ({ payload }) => {
                draggingSurfaces.update((s) => {
                    const next = new Set(s);
                    next.add(payload.surface_id);
                    return next;
                });
            },
        ),
        listen<{ surface_id: number }>(
            "lunaris://window-drag-end",
            ({ payload }) => {
                draggingSurfaces.update((s) => {
                    if (s.has(payload.surface_id)) {
                        const next = new Set(s);
                        next.delete(payload.surface_id);
                        return next;
                    }
                    return s;
                });
            },
        ),
    ];

    const disposer = makeDisposer(pending);
    teardown = () => { disposer(); started = false; teardown = null; };
    return teardown;
}
