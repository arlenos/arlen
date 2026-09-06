/// Accessibility settings store.
///
/// Magnifier settings live in `compositor.toml [accessibility_zoom]`
/// and flow through the existing compositor config-store. Color
/// filter + invert live in a separate state file
/// (`~/.local/state/cosmic-comp/a11y_screen_filter.ron`) and go
/// through the dedicated `accessibility_filter_set/get` commands.

import { derived, writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { compositor } from "./workspaces";
import { t } from "$lib/i18n/messages";
export { compositor };

export type ZoomMovement = "OnEdge" | "Centered" | "Continuously";

export interface AccessibilityZoomConfig {
  start_on_login?: boolean;
  show_overlay?: boolean;
  increment?: number;
  view_moves?: ZoomMovement;
  enable_mouse_zoom_shortcuts?: boolean;
}

export const ZOOM_DEFAULTS: Required<AccessibilityZoomConfig> = {
  start_on_login: false,
  show_overlay: true,
  increment: 50,
  view_moves: "Continuously",
  enable_mouse_zoom_shortcuts: true,
};

/// Derived: these feed a generic select, and the `value` side is the compositor's
/// own enum, so only the label moves.
export const zoomMovementOptions = derived(t, ($t) => [
  { value: "Continuously" as ZoomMovement, label: $t("s.a11y.zoom.continuously") },
  { value: "OnEdge" as ZoomMovement, label: $t("s.a11y.zoom.onEdge") },
  { value: "Centered" as ZoomMovement, label: $t("s.a11y.zoom.centered") },
]);

/// Color filter labels mirror compositor `ColorFilter` variant
/// names. The dedicated `accessibility_filter_set` command maps
/// these strings to the on-disk RON enum.
export type ColorFilterLabel =
  | "None"
  | "Greyscale"
  | "Protanopia"
  | "Deuteranopia"
  | "Tritanopia";

/// Visible labels include a colloquial hint for the colour-
/// blindness filters so the user can pick the right one without
/// medical knowledge.
export const colorFilterOptions = derived(t, ($t) => [
    { value: "None" as ColorFilterLabel, label: $t("s.a11y.filter.none") },
    { value: "Greyscale" as ColorFilterLabel, label: $t("s.a11y.filter.greyscale") },
    { value: "Protanopia" as ColorFilterLabel, label: $t("s.a11y.filter.protanopia") },
    { value: "Deuteranopia" as ColorFilterLabel, label: $t("s.a11y.filter.deuteranopia") },
    { value: "Tritanopia" as ColorFilterLabel, label: $t("s.a11y.filter.tritanopia") },
  ]);

export interface ScreenFilterState {
  inverted: boolean;
  /// `null` ⇒ no filter (mapped to `Option::None` server-side).
  colorFilter: ColorFilterLabel | null;
}

interface FilterStoreState {
  data: ScreenFilterState;
  loading: boolean;
  error: string | null;
  /// True when the last WRITE was refused, kept apart from `error`.
  ///
  /// The same defect the config store carried, in full: the catch put the reason
  /// in `error` and then called `loadFilter()` to roll the switch back, and
  /// `loadFilter()` clears `error` on its first line. So a refused write erased
  /// its own record, and the person saw the invert-colours switch slide back on
  /// its own with nothing said. Had it survived, this page renders `error`
  /// through `ConfigUnavailable` - "these settings cannot be read, the values
  /// below are defaults" - which after a successful rollback is false twice.
  writeFailed: boolean;
}

const inner = writable<FilterStoreState>({
  data: { inverted: false, colorFilter: null },
  loading: false,
  error: null,
  writeFailed: false,
});

export const screenFilter: Readable<FilterStoreState> = {
  subscribe: inner.subscribe,
};

export async function loadFilter(): Promise<void> {
  // `writeFailed` is deliberately NOT cleared: this also runs as the rollback
  // for a refused write, and clearing it there is what erased the failure.
  inner.update((s) => ({ ...s, loading: true, error: null }));
  try {
    const dto = await invoke<{
      inverted: boolean;
      colorFilter?: string | null;
    }>("accessibility_filter_get");
    inner.update((s) => ({
      ...s,
      data: {
        inverted: dto.inverted,
        colorFilter: (dto.colorFilter as ColorFilterLabel | null) ?? null,
      },
      loading: false,
      error: null,
    }));
  } catch (e) {
    inner.update((s) => ({ ...s, loading: false, error: String(e) }));
  }
}

export async function setInverted(value: boolean): Promise<void> {
  inner.update((st) => ({ ...st, writeFailed: false }));
  // Optimistic UI — read current state, mutate, write.
  let cur: ScreenFilterState = { inverted: false, colorFilter: null };
  inner.update((s) => {
    cur = { ...s.data, inverted: value };
    return { ...s, data: cur };
  });
  try {
    await invoke("accessibility_filter_set", {
      dto: {
        inverted: cur.inverted,
        colorFilter: cur.colorFilter,
      },
    });
  } catch {
    await loadFilter();
    inner.update((s) => ({ ...s, writeFailed: true }));
  }
}

export async function setColorFilter(value: ColorFilterLabel): Promise<void> {
  inner.update((st) => ({ ...st, writeFailed: false }));
  let cur: ScreenFilterState = { inverted: false, colorFilter: null };
  inner.update((s) => {
    cur = { ...s.data, colorFilter: value === "None" ? null : value };
    return { ...s, data: cur };
  });
  try {
    await invoke("accessibility_filter_set", {
      dto: {
        inverted: cur.inverted,
        colorFilter: cur.colorFilter,
      },
    });
  } catch {
    await loadFilter();
    inner.update((s) => ({ ...s, writeFailed: true }));
  }
}
