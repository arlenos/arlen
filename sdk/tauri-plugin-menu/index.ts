/**
 * @lunaris/tauri-plugin-menu
 *
 * TypeScript bindings for the Lunaris titlebar protocol.
 * Apps declare titlebar content (tabs, buttons, breadcrumbs);
 * the compositor decides rendering based on window mode.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types ────────────────────────────────────────────────────────────────────

export type TabStatus = "normal" | "modified" | "pinned";
export type ButtonPosition = "left" | "right";
export type CenterContent = "none" | "tabs" | "search" | "segmented";
export type WindowMode = "floating" | "tiled" | "fullscreen" | "frameless";

export interface Tab {
  id: string;
  title: string;
  icon?: string;
  status?: TabStatus;
}

export interface TitlebarButton {
  id: string;
  icon: string;
  tooltip?: string;
  position?: ButtonPosition;
}

export interface BreadcrumbSegment {
  label: string;
  action?: string;
}

// ── Internal helpers ─────────────────────────────────────────────────────────

function statusToU32(status?: TabStatus): number {
  switch (status) {
    case "modified":
      return 1;
    case "pinned":
      return 2;
    default:
      return 0;
  }
}

function positionToU32(position?: ButtonPosition): number {
  return position === "left" ? 0 : 1;
}

function contentToU32(content: CenterContent): number {
  switch (content) {
    case "tabs":
      return 1;
    case "search":
      return 2;
    case "segmented":
      return 3;
    default:
      return 0;
  }
}

// ── Title ────────────────────────────────────────────────────────────────────

/** Set the window title displayed in the titlebar. */
export async function setTitle(title: string): Promise<void> {
  await invoke("plugin:lunaris-menu|set_title", { title });
}

// ── Breadcrumb ───────────────────────────────────────────────────────────────

/** Set breadcrumb navigation segments. */
export async function setBreadcrumb(
  segments: BreadcrumbSegment[]
): Promise<void> {
  await invoke("plugin:lunaris-menu|set_breadcrumb", {
    segmentsJson: JSON.stringify(segments),
  });
}

// ── Center content ───────────────────────────────────────────────────────────

/** Choose what fills the center region of the titlebar. */
export async function setCenterContent(
  content: CenterContent
): Promise<void> {
  await invoke("plugin:lunaris-menu|set_center_content", {
    content: contentToU32(content),
  });
}

// ── Tabs ─────────────────────────────────────────────────────────────────────

/** Add a tab to the titlebar. Replaces existing tab with same ID. */
export async function addTab(tab: Tab): Promise<void> {
  await invoke("plugin:lunaris-menu|add_tab", {
    tab: {
      id: tab.id,
      title: tab.title,
      icon: tab.icon ?? null,
      status: statusToU32(tab.status),
    },
  });
}

/** Remove a tab by ID. */
export async function removeTab(id: string): Promise<void> {
  await invoke("plugin:lunaris-menu|remove_tab", { id });
}

/** Update a tab's title and status. */
export async function updateTab(
  id: string,
  title: string,
  status?: TabStatus
): Promise<void> {
  await invoke("plugin:lunaris-menu|update_tab", {
    id,
    title,
    status: statusToU32(status),
  });
}

/** Set the active (focused) tab. */
export async function activateTab(id: string): Promise<void> {
  await invoke("plugin:lunaris-menu|activate_tab", { id });
}

/** Reorder tabs. Pass tab IDs in the desired order. */
export async function reorderTabs(ids: string[]): Promise<void> {
  await invoke("plugin:lunaris-menu|reorder_tabs", {
    idsJson: JSON.stringify(ids),
  });
}

/** Convenience: set all tabs at once (replaces existing tabs). */
export async function setTabs(tabs: Tab[]): Promise<void> {
  // The protocol has no batch operation; send individual add_tab requests.
  // A future protocol version may add a batch request.
  for (const tab of tabs) {
    await addTab(tab);
  }
  if (tabs.length > 0) {
    await activateTab(tabs[0].id);
  }
}

// ── Buttons ──────────────────────────────────────────────────────────────────

/** Add a custom button to the titlebar. */
export async function addButton(button: TitlebarButton): Promise<void> {
  await invoke("plugin:lunaris-menu|add_button", {
    id: button.id,
    icon: button.icon,
    tooltip: button.tooltip ?? "",
    position: positionToU32(button.position),
  });
}

/** Remove a custom button by ID. */
export async function removeButton(id: string): Promise<void> {
  await invoke("plugin:lunaris-menu|remove_button", { id });
}

/** Enable or disable a button (greyed out when disabled). */
export async function setButtonEnabled(
  id: string,
  enabled: boolean
): Promise<void> {
  await invoke("plugin:lunaris-menu|set_button_enabled", { id, enabled });
}

// ── Search mode ──────────────────────────────────────────────────────────────

/** Enter or leave search mode. */
export async function setSearchMode(enabled: boolean): Promise<void> {
  await invoke("plugin:lunaris-menu|set_search_mode", { enabled });
}

// ── Events (compositor -> app) ───────────────────────────────────────────────

/** The compositor changed the window mode (floating, tiled, fullscreen, frameless). */
export function onModeChanged(
  callback: (mode: WindowMode) => void
): Promise<UnlistenFn> {
  return listen<string>("lunaris-titlebar://mode-changed", ({ payload }) => {
    callback(payload as WindowMode);
  });
}

/** The user clicked a tab in the compositor-rendered titlebar. */
export function onTabActivated(
  callback: (id: string) => void
): Promise<UnlistenFn> {
  return listen<string>("lunaris-titlebar://tab-activated", ({ payload }) => {
    callback(payload);
  });
}

/** The user closed a tab via the titlebar close button. */
export function onTabClosed(
  callback: (id: string) => void
): Promise<UnlistenFn> {
  return listen<string>("lunaris-titlebar://tab-closed", ({ payload }) => {
    callback(payload);
  });
}

/** The user reordered tabs via drag in the titlebar. */
export function onTabReordered(
  callback: (ids: string[]) => void
): Promise<UnlistenFn> {
  return listen<string>("lunaris-titlebar://tab-reordered", ({ payload }) => {
    try {
      callback(JSON.parse(payload));
    } catch {
      // Ignore malformed JSON.
    }
  });
}

/** The user clicked a custom button. */
export function onButtonClicked(
  callback: (id: string) => void
): Promise<UnlistenFn> {
  return listen<string>("lunaris-titlebar://button-clicked", ({ payload }) => {
    callback(payload);
  });
}

/** The user clicked a breadcrumb segment. */
export function onBreadcrumbClicked(
  callback: (index: number, action: string | null) => void
): Promise<UnlistenFn> {
  return listen<{ index: number; action: string | null }>(
    "lunaris-titlebar://breadcrumb-clicked",
    ({ payload }) => {
      callback(payload.index, payload.action);
    }
  );
}

/** The user typed in the search field. */
export function onSearchChanged(
  callback: (query: string) => void
): Promise<UnlistenFn> {
  return listen<string>("lunaris-titlebar://search-changed", ({ payload }) => {
    callback(payload);
  });
}

/** Compositor intercepted a titlebar keyboard shortcut (e.g. Ctrl+Tab). */
export function onKeyboardAction(
  callback: (action: string) => void
): Promise<UnlistenFn> {
  return listen<string>(
    "lunaris-titlebar://keyboard-action",
    ({ payload }) => {
      callback(payload);
    }
  );
}
