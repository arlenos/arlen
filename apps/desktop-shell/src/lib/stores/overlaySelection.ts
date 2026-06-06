/// Multi-selection state for the Workspace Overlay.
///
/// Dedicated store so the selection set survives across component
/// re-renders (HMR, `$derived` recomputes) and so the WorkspaceIndicator
/// can read/write it without prop drilling into every card. Backed by
/// a `Set<string>` internally, exposed as a `writable<Set<string>>`
/// for Svelte subscriptions — mutations re-assign the set by
/// identity so reactivity fires (writable stores compare by
/// reference, not Set.has equality).
///
/// Lifecycle rules (see spec §Edge Cases):
/// - Overlay close clears selection
/// - Workspace-nav (Arrow/Tab inside overlay) clears selection
/// - Escape clears selection
/// - Windows closed externally drop out of the selection
///   automatically because we filter against the live `windows`
///   store on every read via `validSelection()`.

import { derived, get, writable, type Readable } from "svelte/store";
import { windows, type WindowInfo } from "./windows.js";

const _selected = writable<Set<string>>(new Set());

export const selectedWindowIds: Readable<Set<string>> = {
    subscribe: _selected.subscribe,
};

/// Derived: selection filtered to ids that still exist in the windows
/// store. Used by the context menu / drag handlers so a window that
/// closed between selection time and action time simply drops out
/// rather than throwing.
export const validSelection: Readable<Set<string>> = derived(
    [_selected, windows],
    ([$sel, $wins]) => {
        const live = new Set($wins.map((w: WindowInfo) => w.id));
        const out = new Set<string>();
        for (const id of $sel) {
            if (live.has(id)) out.add(id);
        }
        return out;
    },
);

export function toggleSelection(windowId: string): void {
    _selected.update((s) => {
        const next = new Set(s);
        if (next.has(windowId)) {
            next.delete(windowId);
        } else {
            next.add(windowId);
        }
        return next;
    });
}

export function addToSelection(windowId: string): void {
    _selected.update((s) => {
        if (s.has(windowId)) return s;
        const next = new Set(s);
        next.add(windowId);
        return next;
    });
}

export function selectOnly(windowId: string): void {
    _selected.set(new Set([windowId]));
}

export function clearSelection(): void {
    _selected.update((s) => (s.size === 0 ? s : new Set()));
}

export function isSelected(windowId: string): boolean {
    return get(_selected).has(windowId);
}

/// Snapshot the current selection (non-reactive). Used by handlers
/// that fire async actions — they want the set frozen at invoke
/// time, not a Svelte-subscribed live view.
export function selectionSnapshot(): string[] {
    return Array.from(get(_selected));
}

/// Prune ids that no longer correspond to live windows. Called from
/// the WorkspaceIndicator on each `windows` tick so closed-externally
/// windows don't linger in the selection set.
export function pruneSelection(liveIds: Set<string>): void {
    _selected.update((s) => {
        let mutated = false;
        const next = new Set<string>();
        for (const id of s) {
            if (liveIds.has(id)) next.add(id);
            else mutated = true;
        }
        return mutated ? next : s;
    });
}
