/// The text-editor app's global menu in the shell's top-left menu bar (the
/// knowledge pattern): the app's own verbs beside its surface, labels reused
/// from the catalogue so nothing drifts. Registered per translator value so a
/// locale switch re-registers; under vite both calls fail silently and the
/// menu is simply absent. Live it needs `arlen-shell:allow-menu-register` /
/// `-unregister` in this app's capabilities file, and the plugin's
/// menu-action return channel - the same routed seams as the pdf reader's.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writable } from "svelte/store";
import type { Translate } from "@arlen/ui-kit/i18n";
import { t } from "$lib/i18n/messages";

const APP_ID = "dev.arlen.text-editor";

/// One entry, in the shape `os_sdk::menu::MenuItem` deserializes.
export type MenuItem = {
  label: string;
  action?: string;
  shortcut?: string;
  type: "item" | "separator";
};
export type MenuGroup = { label: string; items: MenuItem[] };

const item = (label: string, action: string, shortcut?: string): MenuItem => ({
  label,
  action,
  shortcut,
  type: "item",
});
const sep = (): MenuItem => ({ label: "", type: "separator" });

/// The menu as the reader's language renders it. Pure, so a test can read the
/// labels without a running app or a shell to publish into.
export function appMenuGroups(t: Translate): MenuGroup[] {
  return [
    {
      label: t("te.menu.file"),
      items: [
        item(t("te.menu.save"), "file.save", "Ctrl+S"),
        sep(),
        item(t("te.print"), "file.print"),
      ],
    },
  ];
}

/// The action a menu click dispatched; the consumer clears it.
export const menuAction = writable<string | null>(null);

/// Register the menu and route dispatched actions into the store.
export async function initAppMenu(): Promise<void> {
  t.subscribe((tr) => {
    void invoke("plugin:arlen-shell|menu_register", { groups: appMenuGroups(tr) }).catch(() => {
      // No shell relay (vite, or the permission seam not landed): absent.
    });
  });
  try {
    await listen<{ app_id: string; action: string }>("arlen://menu-action", ({ payload }) => {
      if (payload.app_id !== APP_ID) return;
      menuAction.set(payload.action);
    });
  } catch {
    // Same seam: without the relay no actions arrive.
  }
}
