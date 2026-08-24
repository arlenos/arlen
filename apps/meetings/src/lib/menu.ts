/// The meetings app's global menu in the shell's top-left menu bar (the
/// knowledge pattern), with the tree's first live checkbox: Transcribe mirrors
/// the capture surface's own toggle, so the menu re-registers whenever the
/// language OR that state changes (the shell holds no state; a checked mark is
/// part of the registered tree). Under vite the calls fail silently and the
/// menu is absent. Live it needs `arlen-shell:allow-menu-register` /
/// `-unregister` in this app's capabilities file, and the plugin's
/// menu-action return channel - the same routed seams as the pdf reader's.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writable } from "svelte/store";
import type { Translate } from "@arlen/ui-kit/i18n";

const APP_ID = "dev.arlen.meetings";

/// One entry, in the shape `os_sdk::menu::MenuItem` deserializes.
export type MenuItem = {
  label: string;
  action?: string;
  shortcut?: string;
  checked?: boolean;
  type: "item" | "separator";
};
export type MenuGroup = { label: string; items: MenuItem[] };

const item = (label: string, action: string): MenuItem => ({ label, action, type: "item" });
const check = (label: string, action: string, checked: boolean): MenuItem => ({
  label,
  action,
  checked,
  type: "item",
});
const sep = (): MenuItem => ({ label: "", type: "separator" });

/// The menu as the reader's language renders it, over the live state it marks.
/// Pure, so a test can read the labels without a running app.
export function appMenuGroups(t: Translate, state: { transcribe: boolean }): MenuGroup[] {
  return [
    {
      label: t("mt.menu.meeting"),
      items: [
        item(t("mt.start"), "meeting.start"),
        item(t("mt.menu.stop"), "meeting.stop"),
        sep(),
        item(t("mt.open"), "meeting.open_editor"),
      ],
    },
    {
      label: t("mt.menu.view"),
      items: [check(t("mt.transcribe"), "view.transcribe", state.transcribe)],
    },
  ];
}

/// Send the current tree. Best-effort; the caller re-invokes on any change.
export async function registerAppMenu(groups: MenuGroup[]): Promise<void> {
  try {
    await invoke("plugin:arlen-shell|menu_register", { groups });
  } catch {
    // No shell relay (vite, or the permission seam not landed): absent.
  }
}

/// The action a menu click dispatched; the consumer clears it.
export const menuAction = writable<string | null>(null);

/// Route dispatched actions into the store.
export async function initMenuActions(): Promise<void> {
  try {
    await listen<{ app_id: string; action: string }>("arlen://menu-action", ({ payload }) => {
      if (payload.app_id !== APP_ID) return;
      menuAction.set(payload.action);
    });
  } catch {
    // Same seam: without the relay no actions arrive.
  }
}
