/// The reader's global menu in the shell's top-left app menu (the knowledge
/// pattern): view and navigation commands live there, not as extra surface
/// buttons - which is what keeps the document-only mode usable. Registered
/// through the shared shell plugin per translator value, so a locale switch
/// re-registers; under vite both calls fail silently and the menu is simply
/// absent. Live it additionally needs `arlen-shell:allow-menu-register` /
/// `-unregister` in this app's capabilities file - a routed seam.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writable } from "svelte/store";
import { t } from "$lib/i18n/messages";

const APP_ID = "dev.arlen.pdf";

/// The action a menu click dispatched; the page consumes and clears it.
export const menuAction = writable<string | null>(null);

const menuFor = (tr: (k: string) => string) => [
  {
    label: tr("pdf.menu.view"),
    items: [
      { label: tr("pdf.documentOnly"), action: "view.document-only", shortcut: ".", type: "item" },
      { label: tr("pdf.showContents"), action: "view.contents", type: "item" },
      { label: "", action: "", type: "separator" },
      { label: tr("pdf.zoomIn"), action: "view.zoom-in", shortcut: "+", type: "item" },
      { label: tr("pdf.zoomOut"), action: "view.zoom-out", shortcut: "-", type: "item" },
      { label: tr("pdf.actualSize"), action: "view.actual-size", shortcut: "0", type: "item" },
      { label: tr("pdf.fitWidth"), action: "view.fit-width", type: "item" },
      { label: tr("pdf.fitPage"), action: "view.fit-page", type: "item" },
    ],
  },
  {
    label: tr("pdf.menu.go"),
    items: [
      { label: tr("pdf.nextPage"), action: "go.next", type: "item" },
      { label: tr("pdf.prevPage"), action: "go.previous", type: "item" },
      { label: "", action: "", type: "separator" },
      { label: tr("pdf.firstPage"), action: "go.first", shortcut: "Home", type: "item" },
      { label: tr("pdf.lastPage"), action: "go.last", shortcut: "End", type: "item" },
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
