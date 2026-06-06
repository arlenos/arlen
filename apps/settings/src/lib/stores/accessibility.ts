/// Accessibility settings store.
///
/// Magnifier settings live in `compositor.toml [accessibility_zoom]`
/// and flow through the existing compositor config-store. Color
/// filter + invert live in a separate state file
/// (`~/.local/state/cosmic-comp/a11y_screen_filter.ron`) and go
/// through the dedicated `accessibility_filter_set/get` commands.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { compositor } from "./workspaces";
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

export const ZOOM_MOVEMENT_OPTIONS: { value: ZoomMovement; label: string }[] = [
  { value: "Continuously", label: "Continuously" },
  { value: "OnEdge", label: "On edge" },
  { value: "Centered", label: "Centered" },
];

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
export const COLOR_FILTER_OPTIONS: { value: ColorFilterLabel; label: string }[] =
  [
    { value: "None", label: "None" },
    { value: "Greyscale", label: "Greyscale" },
    { value: "Protanopia", label: "Protanopia (red weakness)" },
    { value: "Deuteranopia", label: "Deuteranopia (green weakness)" },
    { value: "Tritanopia", label: "Tritanopia (blue weakness)" },
  ];

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
