/// The calendar's global menu in the shell's top-left app menu (the knowledge
/// pattern, same as the pdf reader): creating, jumping and view switching live
/// there beside the surface's own controls. Registered through the shared
/// shell plugin per translator value, so a locale switch re-registers; under
/// vite both calls fail silently and the menu is simply absent. Live it needs
/// `arlen-shell:allow-menu-register` / `-unregister` in this app's
/// capabilities file - the same routed seam as the pdf reader's.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writable } from "svelte/store";
import { t } from "$lib/i18n/messages";

const APP_ID = "dev.arlen.calendar";

/// The action a menu click dispatched; the page consumes and clears it.
export const menuAction = writable<string | null>(null);

const menuFor = (tr: (k: string) => string) => [
  {
    label: tr("cal.menu.event"),
    items: [{ label: tr("cal.newEvent"), action: "event.new", shortcut: "C", type: "item" }],
  },
  {
    label: tr("cal.menu.view"),
    items: [
      { label: tr("cal.view.week"), action: "view.week", shortcut: "W", type: "item" },
      { label: tr("cal.view.threeDays"), action: "view.three", shortcut: "X", type: "item" },
      { label: tr("cal.view.month"), action: "view.month", shortcut: "M", type: "item" },
      { label: tr("cal.view.day"), action: "view.day", shortcut: "D", type: "item" },
      { label: tr("cal.view.agenda"), action: "view.agenda", shortcut: "A", type: "item" },
    ],
  },
  {
    label: tr("cal.menu.go"),
    items: [
      { label: tr("cal.todayButton"), action: "go.today", shortcut: "T", type: "item" },
      { label: "", action: "", type: "separator" },
      { label: tr("cal.prev"), action: "go.back", type: "item" },
      { label: tr("cal.next"), action: "go.forward", type: "item" },
    ],
  },
];

/// Register the menu and route dispatched actions into the store.
export async function initAppMenu(): Promise<void> {
  t.subscribe((tr) => {
    void invoke("plugin:arlen-shell|menu_register", { groups: menuFor(tr) }).catch(() => {
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
