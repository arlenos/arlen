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
}

const inner = writable<FilterStoreState>({
  data: { inverted: false, colorFilter: null },
  loading: false,
  error: null,
});

export const screenFilter: Readable<FilterStoreState> = {
  subscribe: inner.subscribe,
};

export async function loadFilter(): Promise<void> {
  inner.update((s) => ({ ...s, loading: true, error: null }));
  try {
    const dto = await invoke<{
      inverted: boolean;
      colorFilter?: string | null;
    }>("accessibility_filter_get");
    inner.set({
      data: {
        inverted: dto.inverted,
        colorFilter: (dto.colorFilter as ColorFilterLabel | null) ?? null,
      },
      loading: false,
      error: null,
    });
  } catch (e) {
    inner.update((s) => ({ ...s, loading: false, error: String(e) }));
  }
}

export async function setInverted(value: boolean): Promise<void> {
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
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    await loadFilter();
  }
}

export async function setColorFilter(value: ColorFilterLabel): Promise<void> {
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
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    await loadFilter();
  }
}
